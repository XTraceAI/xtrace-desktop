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
    /// Native Claude file membership may reference an identical work record.
    /// Never available to plugin receipts or generic transcript imports.
    pub native_history: bool,
    /// A pinned Codex reader may classify inherited context and replace legacy
    /// UI counters when a native response ledger first identifies the request.
    pub native_codex: bool,
    /// Per-input native adapter proof; never accepted without native_history.
    pub confirmed_iteration_usage: &'a [bool],
    /// Per-input source proof that Codex injected that input, from the same
    /// exact read as the record; empty means none, otherwise exactly one entry
    /// per input. Only native Codex reader history may carry one, and a proof
    /// is kept only when its own input inserts the record in this batch. See
    /// [`crate::injected`].
    pub injected_context: &'a [Option<crate::injected::InjectedContextProof>],
    /// Per-input marker that the native line itself says Claude Code wrote it
    /// as a task notification (`origin.kind`); empty means none, otherwise
    /// exactly one entry per input. A marked input that is accepted binds a
    /// proof to its stored record, new or already held. See
    /// `task_notification.rs`.
    pub task_notifications: &'a [bool],
    /// Per-input marker that the caller knows a human-input adjustment says
    /// only part, or none, of this input is a person's words, so no person
    /// preview may be kept for it; empty means none, otherwise exactly one
    /// entry per input. See `record_preview.rs`.
    pub withheld_previews: &'a [bool],
    pub namespace: Option<&'a str>,
    /// Empty means unknown; otherwise exactly one native identity per input.
    pub identities: &'a [crate::model::RecordIdentity],
    pub session_sources: &'a [SessionSourceObservation],
    pub record_sources: &'a [RecordSourceObservation],
    /// Structural tool events this batch covers: slash commands and hook
    /// summaries, which are not assistant tool calls and belong to no record.
    /// They commit with the rows and the checkpoint, never beside them, and a
    /// conflicting replay of one fails the whole batch.
    pub tool_events: &'a [crate::ingest::ToolEvent],
    /// Exact pull-request witnesses read from the same native input, only for
    /// discovered Claude history. A link naming this batch's session commits
    /// with the rows and checkpoint or fails the whole batch. A link naming
    /// another session is a fork's inherited copy of that session's evidence:
    /// it attaches to that session only when the index already holds it as a
    /// Claude session, and otherwise attaches nowhere, never to this batch's
    /// session in its place.
    pub pr_links: &'a [crate::pr_link::PrLinkObservation],
    pub receipt: Option<SubmittedReceipt<'a>>,
    /// Exact retry matching happens under this batch's write transaction.
    pub receipt_replay: ReceiptReplay,
    /// Writer mode: rejected identities contribute no source or receipt evidence.
    pub evidence_policy: EvidencePolicy,
    pub cursor: Option<&'a SourceCursor>,
    /// A discovered identity that fills under this batch's transaction, so a
    /// rejected batch leaves no label behind.
    pub discovery: Option<&'a crate::ingest::DiscoveredSession>,
    /// Resume progress that commits with this batch's rows and never without
    /// them: the position through the input these rows came from, bound to
    /// the source generation it was read under. A batch without records
    /// cannot carry one; progress no rows carry is recorded on its own.
    pub checkpoint: Option<&'a NativeCheckpoint>,
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
            native_history: false,
            native_codex: false,
            confirmed_iteration_usage: &[],
            injected_context: &[],
            task_notifications: &[],
            withheld_previews: &[],
            namespace: None,
            identities: &[],
            session_sources: &[],
            record_sources: &[],
            tool_events: &[],
            pr_links: &[],
            receipt: None,
            receipt_replay: ReceiptReplay::Reject,
            evidence_policy: EvidencePolicy::RequireAll,
            cursor: None,
            discovery: None,
            checkpoint: None,
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
/// Initial native imports retain locators at position zero; generation-aware
/// incremental positions belong to the eventual incremental consumer.
/// A native resume checkpoint: how far a source was consumed, bound to the
/// generation of the source it was read from. `generation` is a JSON object
/// the importer defines (a file identity and prefix digest, or a scan
/// instant); the store keeps it opaque. Unlike a plugin cursor, a checkpoint
/// may move backwards: a replaced or truncated source starts a new generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeCheckpoint {
    pub source: SessionSource,
    pub cursor_key: String,
    pub generation: String,
    pub position: i64,
    pub updated_at: i64,
}

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
    /// One result per supplied injected context proof, in input order.
    pub injected_context: Vec<crate::injected::InjectedContextOutcome>,
}

impl Store {
    /// Apply one fixed batch under one immediate transaction. Any validation,
    /// write or commit failure returns no successful outcome and rolls back all
    /// participating facts. There is no callback, nested commit or event hook.
    pub fn apply_ingest_batch(&mut self, batch: &IngestBatch<'_>) -> Result<IngestBatchOutcome> {
        write::validate_session(batch.session)?;
        if batch.native_history
            && (batch.session.host != crate::Host::Claude
                || batch.session.source != SessionSource::Transcript
                || batch.discovery.is_none()
                || batch.receipt.is_some())
        {
            return Err(Error::InvalidInput(
                "native copies require discovered Claude history",
            ));
        }
        if batch.native_codex
            && (batch.session.host != crate::Host::Codex
                || batch.session.source != SessionSource::ReadersCli
                || batch.discovery.is_none()
                || batch.receipt.is_some())
        {
            return Err(Error::InvalidInput(
                "native Codex evidence requires discovered reader history",
            ));
        }
        if !batch.pr_links.is_empty()
            && (!batch.native_history
                || batch
                    .pr_links
                    .iter()
                    .any(|link| link.confidence != crate::pr_link::PrConfidence::Exact))
        {
            return Err(Error::InvalidInput(
                "native PR witnesses require discovered Claude history and exact evidence",
            ));
        }
        if !batch.confirmed_iteration_usage.is_empty()
            && (!batch.native_history
                || batch.confirmed_iteration_usage.len() != batch.records.len())
        {
            return Err(Error::InvalidInput(
                "iteration proofs require aligned native inputs",
            ));
        }
        crate::injected::validate(batch)?;
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
                .tool_events
                .iter()
                .any(|event| event.session_id != batch.session.session_id)
            || batch
                .receipt
                .as_ref()
                .is_some_and(|submitted| submitted.receipt.session_id != batch.session.session_id)
        {
            return Err(Error::InvalidInput(
                "batch facts must belong to its canonical session",
            ));
        }
        // Structural events and PR witnesses are rows this batch covers too: a
        // stretch of input whose only storable lines were hook summaries or
        // witnesses still carries progress.
        if batch.checkpoint.is_some()
            && batch.records.is_empty()
            && batch.tool_events.is_empty()
            && batch.pr_links.is_empty()
        {
            return Err(Error::InvalidInput(
                "a checkpoint commits only with the rows it covers",
            ));
        }
        if batch.discovery.is_some_and(|discovery| {
            discovery.host != batch.session.host
                || discovery
                    .conversation_id
                    .as_deref()
                    .is_some_and(|id| id != batch.session.session_id)
                || batch.session.native_session_id.as_deref()
                    != Some(discovery.native_session_id.as_str())
                || discovery
                    .surface
                    .as_ref()
                    .zip(batch.session.surface.as_ref())
                    .is_some_and(|(a, b)| a != b)
                || discovery
                    .started_at_ms
                    .zip(batch.session.started_at_ms)
                    .is_some_and(|(a, b)| a != b)
        }) {
            return Err(Error::InvalidInput(
                "discovery must belong to the batch session",
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
            if batch.native_history {
                Some(crate::Host::Claude)
            } else if batch.native_codex {
                Some(crate::Host::Codex)
            } else {
                None
            },
            batch.confirmed_iteration_usage,
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
        // Judged from this transaction's own outcomes and stored rows, never
        // from a record this input did not insert. Native Codex batches carry
        // no receipt, so none of this is inside the provisional savepoint.
        outcome.injected_context = crate::injected::apply(&transaction, batch, &outcome.records)?;
        crate::task_notification::apply(
            &transaction,
            batch,
            &outcome.records,
            &mut outcome.affected_owners,
        )?;
        // After every proof that can change a record's eligibility in this
        // batch, inside the destination savepoint, so a rolled-back or
        // rejected input keeps no preview.
        crate::record_preview::apply(&transaction, batch, &outcome.records)?;
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
        // Inside the destination savepoint: a batch whose rows are all rolled
        // back leaves no structural event behind either. A retry of an event
        // already stored is recognised, and a conflicting one fails the batch.
        // The event is stored under the session that owns its identity, so a
        // copied native context adds no second row.
        for event in batch
            .tool_events
            .iter()
            .filter(|event| !rejected.contains(event.source_event_id.as_str()))
        {
            // A slash command's identity is its user record's, so a record this
            // batch could not store states nothing here, whoever else owns the
            // UUID. Reading the owner instead would let a rejected occurrence
            // write a command onto a session that never stored that record: the
            // rejection is exactly the evidence that this input is not a
            // trustworthy account of it. An accepted native copy is not
            // rejected, so it still resolves to the canonical record's owner.
            ingest::insert_tool_event(&transaction, event)?;
        }
        // Witnesses commit beside the rows and the checkpoint covering them; a
        // conflicting pull-request row fails the whole batch. An inherited copy
        // is the named session's evidence: it never becomes this session's
        // link, and a session the index does not hold as Claude history gets
        // none, so replaying the copy before or after the original converges.
        for link in batch.pr_links {
            if link.session_id != batch.session.session_id
                && transaction
                    .query_row(
                        "SELECT 1 FROM sessions WHERE session_id=?1 AND host='claude'",
                        [&link.session_id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_none()
            {
                continue;
            }
            crate::pr_link::record_pr_link(&transaction, link)?;
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
        // Progress advances only past input every record of which was stored;
        // a batch with a rejected or dropped record keeps the earlier checkpoint
        // (and a batch without records was refused above).
        if let Some(checkpoint) = batch.checkpoint
            && outcome
                .records
                .iter()
                .all(|row| row.disposition.is_accepted())
        {
            upsert_native_checkpoint(&transaction, checkpoint)?;
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
    /// Retain a native source locator without claiming an incremental resume
    /// position. Initial imports always read from the beginning. Empty scans
    /// may reset an existing locator but never create a new one.
    pub fn record_native_source_locator(
        &mut self,
        cursor: &SourceCursor,
        create: bool,
    ) -> Result<()> {
        if !matches!(
            cursor.source,
            SessionSource::Transcript | SessionSource::ReadersCli
        ) {
            return Err(Error::InvalidInput(
                "native locator requires a native source",
            ));
        }
        self.connection.execute(
            "INSERT INTO source_cursors(source,cursor_key,position,updated_at)
             SELECT ?1,?2,0,?3 WHERE ?4 OR EXISTS (
                 SELECT 1 FROM source_cursors WHERE source=?1 AND cursor_key=?2)
             ON CONFLICT(source,cursor_key) DO UPDATE SET position=0,
                 updated_at=max(source_cursors.updated_at,excluded.updated_at)",
            params![cursor.source, cursor.cursor_key, cursor.updated_at, create],
        )?;
        Ok(())
    }
}

impl Store {
    /// The checkpoint recorded for a native source, if any.
    pub fn native_checkpoint(
        &self,
        source: SessionSource,
        key: &str,
    ) -> Result<Option<NativeCheckpoint>> {
        Ok(self
            .connection
            .query_row(
                "SELECT source,cursor_key,generation,position,updated_at FROM native_checkpoints
                 WHERE source=?1 AND cursor_key=?2",
                params![source, key],
                |row| {
                    Ok(NativeCheckpoint {
                        source: row.get(0)?,
                        cursor_key: row.get(1)?,
                        generation: row.get(2)?,
                        position: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// Record a checkpoint on its own, after the rows it covers have committed
    /// (a scan that consumed input without producing a new row, say).
    pub fn record_native_checkpoint(&mut self, checkpoint: &NativeCheckpoint) -> Result<()> {
        upsert_native_checkpoint(&self.connection, checkpoint)
    }

    /// Forget a checkpoint, so the next scan reads the source from the start.
    pub fn clear_native_checkpoint(&mut self, source: SessionSource, key: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM native_checkpoints WHERE source=?1 AND cursor_key=?2",
            params![source, key],
        )?;
        Ok(())
    }
}

fn upsert_native_checkpoint(connection: &Connection, checkpoint: &NativeCheckpoint) -> Result<()> {
    if !matches!(
        checkpoint.source,
        SessionSource::Transcript | SessionSource::ReadersCli
    ) {
        return Err(Error::InvalidInput(
            "native checkpoint requires a native source",
        ));
    }
    connection.execute(
        "INSERT INTO native_checkpoints(source,cursor_key,generation,position,updated_at)
         VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(source,cursor_key) DO UPDATE SET generation=excluded.generation,
             position=excluded.position, updated_at=excluded.updated_at",
        params![
            checkpoint.source,
            checkpoint.cursor_key,
            checkpoint.generation,
            checkpoint.position,
            checkpoint.updated_at
        ],
    )?;
    Ok(())
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

/// Escape a literal value for use inside a `LIKE` pattern whose `ESCAPE` is
/// `\`. A locator key holds a local path, whose characters are the caller's,
/// not a pattern: an identifier containing `%` or `_` must match itself.
pub fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// How many `LIKE` patterns one locator lookup may carry. A host names a
/// session's sources with a handful of layouts; an unbounded list would make
/// the statement, not the caller, decide how much the query costs.
pub const MAX_LOCATOR_PATTERNS: usize = 8;

/// The most rows one bounded locator lookup returns. A leading-wildcard
/// pattern can match far more keys than the caller will keep; this bounds what
/// is materialized for one lookup, not every step SQLite takes to find them.
pub const MAX_LOCATOR_ROWS: usize = 256;

/// What a bounded locator lookup found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocatorRows {
    /// Every matching row, at most [`MAX_LOCATOR_ROWS`] of them.
    Complete(Vec<SourceCursor>),
    /// More rows match than one lookup returns. None is returned: a prefix of
    /// the matches is not the set, and a row past it could be the one wanted.
    Saturated,
}

impl Store {
    /// The native source locators of one `source` whose key matches any of
    /// `patterns` (`LIKE` with `\` as its escape; see [`escape_like`]).
    ///
    /// This reads `source_cursors` only: a locator names a local path the
    /// index read, never content. Callers own the host layout the patterns
    /// describe, so no session-to-path rule is duplicated here.
    pub fn source_cursors_like(
        &self,
        source: SessionSource,
        patterns: &[&str],
    ) -> Result<Vec<SourceCursor>> {
        self.cursors_like(source, patterns, None)
    }

    /// [`Self::source_cursors_like`], materializing at most one row past
    /// [`MAX_LOCATOR_ROWS`] so that a lookup matching more says so rather than
    /// returning them all or an arbitrary prefix.
    pub fn source_cursors_like_bounded(
        &self,
        source: SessionSource,
        patterns: &[&str],
    ) -> Result<LocatorRows> {
        let rows = self.cursors_like(source, patterns, Some(MAX_LOCATOR_ROWS + 1))?;
        Ok(if rows.len() > MAX_LOCATOR_ROWS {
            LocatorRows::Saturated
        } else {
            LocatorRows::Complete(rows)
        })
    }

    fn cursors_like(
        &self,
        source: SessionSource,
        patterns: &[&str],
        limit: Option<usize>,
    ) -> Result<Vec<SourceCursor>> {
        if patterns.is_empty() || patterns.len() > MAX_LOCATOR_PATTERNS {
            return Err(Error::InvalidInput(
                "a locator lookup needs one to MAX_LOCATOR_PATTERNS patterns",
            ));
        }
        let mut sql = String::from(
            "SELECT source,cursor_key,position,updated_at FROM source_cursors WHERE source=?1 AND (",
        );
        for index in 0..patterns.len() {
            if index > 0 {
                sql.push_str(" OR ");
            }
            sql.push_str("cursor_key LIKE ?");
            sql.push_str(&(index + 2).to_string());
            sql.push_str(" ESCAPE '\\'");
        }
        sql.push_str(") ORDER BY cursor_key");
        let mut parameters: Vec<rusqlite::types::Value> = vec![source.as_str().to_owned().into()];
        parameters.extend(patterns.iter().map(|pattern| (*pattern).to_owned().into()));
        if let Some(limit) = limit {
            sql.push_str(" LIMIT ?");
            sql.push_str(&(parameters.len() + 1).to_string());
            parameters.push(i64::try_from(limit).unwrap_or(i64::MAX).into());
        }
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(parameters), |row| {
            Ok(SourceCursor {
                source: row.get(0)?,
                cursor_key: row.get(1)?,
                position: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
