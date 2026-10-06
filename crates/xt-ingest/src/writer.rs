//! Synchronous ingestion composition. Returned acknowledgements and events exist
//! only after the single store transaction commits; adapters publish them later.

use crate::{
    canonical::{ParsedRecord, PrLink, SourceContext, StopHookSummary},
    tool_use,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use xt_store::{
    Error, Host, Result, SessionMeta, SessionSource, Store, StoredRecord,
    batch::{
        EvidencePolicy, IngestBatch, ReceiptReplay, RecordDisposition, SourceCursor,
        SubmittedReceipt,
    },
    ingest::{CaptureReceipt, RecordCoverage, RecordSourceObservation, SessionSourceObservation},
    injected::{InjectedContextOutcome, InjectedContextProof},
    measurement::{Projection, SCHEMA_VERSION},
    model::RecordIdentity,
    pr_link::{PrConfidence, PrIdentity, PrLinkObservation},
};

pub const MAX_BATCH_RECORDS: usize = 2_000;

pub struct WriteBatch<'a> {
    pub context: &'a SourceContext,
    /// A known adapter host need not manufacture an absent raw platform label.
    pub declared_host: Option<Host>,
    pub records: &'a [ParsedRecord],
    /// Native stop-hook summaries read from the same input as `records`. They
    /// are structural events, not records: they carry no content, never enter a
    /// record's tool-call count, and commit with this batch or not at all.
    pub hook_summaries: &'a [StopHookSummary],
    /// Native Claude `pr-link` witnesses read from the same input: exact
    /// evidence, never inferred, reconciled by the same identity rules.
    pub pr_witnesses: &'a [PrWitness],
    pub title: Option<&'a str>,
    /// Session facts a source header states outside its records (a reader's
    /// `cwd`/`git_branch`); merged with fill semantics, conflicts are flagged.
    pub cwd: Option<&'a str>,
    pub git_branch: Option<&'a str>,
    pub namespace: Option<&'a str>,
    pub keep_content: bool,
    pub observed_at: i64,
    /// Stable parent facts reused verbatim on retry. Only Plugin accepts receipts.
    pub receipt: Option<&'a CaptureReceipt>,
    pub cursor: Option<&'a SourceCursor>,
    /// A discovered identity that fills only if this batch commits.
    pub discovery: Option<&'a xt_store::ingest::DiscoveredSession>,
    /// Resume progress that commits only with this batch's rows.
    pub checkpoint: Option<&'a xt_store::batch::NativeCheckpoint>,
}

/// One explicit native `pr-link` line. The adapter has reconciled the line's
/// own identity labels and bound them to its file's session context, exactly as
/// it does a record's, so the writer checks them against the batch like any
/// other observation. `named_session` is the native session the line itself
/// names: the batch's own for a witness written there, another for a fork's
/// inherited copy, whose evidence belongs to the session it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrWitness {
    pub link: PrLink,
    pub named_session: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchOutcome {
    pub conversation_id: String,
    pub records_new: usize,
    pub records_enriched: usize,
    pub records_dropped: usize,
    pub ack_through: Option<String>,
    pub dropped_reasons: Vec<(usize, RecordDisposition)>,
    pub events: Vec<ChangeEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeEvent {
    pub conversation_id: String,
    pub source: SessionSource,
    pub surface: Option<String>,
    pub records_new: usize,
    pub records_enriched: usize,
    pub invalidate_measurements: bool,
    pub invalidate_cost: bool,
    pub backfill_position: Option<i64>,
}

pub fn coverage(session: &str, record: &ParsedRecord) -> Result<RecordCoverage> {
    let identity = identity(record, 0);
    let projection = Projection::from_canonical(session, &record.canonical, &identity)?;
    Ok(covered(
        record.canonical.uuid.as_deref().unwrap(),
        &projection,
    ))
}

/// A field/revision comparison, not a complete session-capture verdict. The
/// caller must separately evaluate source identity, conflicts and representatives.
///
/// Coverage is compared under the digest schema version it was sealed with, so
/// an older receipt is neither rewritten nor reinterpreted. A version that
/// cannot describe the current record — an unknown one, or version 1 for an
/// input a confirmation has since reclassified — never matches.
pub fn matches_current(coverage: &RecordCoverage, record: &StoredRecord) -> Result<bool> {
    let projection = Projection::from_stored(record)?;
    Ok(!record.has_conflict
        && covered_as(&record.uuid, &projection, coverage.digest_schema_version)
            .is_some_and(|current| coverage == &current))
}

fn covered(uuid: &str, projection: &Projection) -> RecordCoverage {
    covered_as(uuid, projection, SCHEMA_VERSION).expect("the current version encodes every record")
}

fn covered_as(uuid: &str, projection: &Projection, version: u32) -> Option<RecordCoverage> {
    let (metric_field_mask, bytes) = projection.encoded(version)?;
    Some(RecordCoverage {
        record_uuid: uuid.to_owned(),
        metric_field_mask,
        measurement_revision: format!("{:x}", Sha256::digest(bytes)),
        digest_schema_version: version,
    })
}

/// The single digest version an existing sealed receipt was recorded with, or
/// the current version for a new receipt. Sealed coverage is immutable, so
/// reading it before the batch transaction cannot race a change. For a version
/// this build cannot encode, or a mixed set, the retry is encoded with the
/// current version, which the store's exact comparison then rejects.
fn sealed_version(store: &Store, receipt_id: &str) -> Result<u32> {
    let sealed = store.capture_coverage(receipt_id)?;
    let mut versions = sealed.iter().map(|item| item.digest_schema_version);
    Ok(match versions.next() {
        Some(first) if versions.all(|version| version == first) => first,
        _ => SCHEMA_VERSION,
    })
}

fn identity(record: &ParsedRecord, observed_at: i64) -> RecordIdentity {
    RecordIdentity {
        parent_uuid: record.native.parent_uuid.clone(),
        agent_id: record.native.agent_id.clone(),
        subtype: record.native.subtype.clone(),
        first_seen_at: Some(observed_at),
    }
}

pub fn write_batch(store: &mut Store, request: &WriteBatch<'_>) -> Result<BatchOutcome> {
    write_batch_proven(store, request, &[], &[]).map(|(outcome, _)| outcome)
}

/// `write_batch` with one optional injected context proof per record of the
/// request, aligned by input index (empty for none). The proofs travel in the
/// same Store transaction as their records; the Store keeps one only where its
/// own input inserted an eligible record, and reports each proof's outcome in
/// input order. A proof the Store refuses fails the whole batch, as any other
/// invalid input does. A request with no proof is exactly `write_batch`.
///
/// `adjusted` marks, aligned the same way (empty for none), each input a
/// human-input adjustment the caller will apply says is only partly, or not
/// at all, a person's words; the Store keeps no person preview of it. An
/// input carrying its own adjustment is marked whatever the caller says.
pub fn write_batch_proven(
    store: &mut Store,
    request: &WriteBatch<'_>,
    proofs: &[Option<InjectedContextProof>],
    adjusted: &[bool],
) -> Result<(BatchOutcome, Vec<InjectedContextOutcome>)> {
    if !adjusted.is_empty() && adjusted.len() != request.records.len() {
        return Err(Error::InvalidInput(
            "adjusted inputs must align with the batch's records",
        ));
    }
    let proofs = if proofs.iter().any(Option::is_some) {
        proofs
    } else {
        &[]
    };
    if request.records.len() > MAX_BATCH_RECORDS || request.pr_witnesses.len() > MAX_BATCH_RECORDS {
        return Err(Error::InvalidInput(
            "ingestion batch exceeds its record bound",
        ));
    }
    let session = resolve_session(request)?;
    let source = session.source;
    if request.receipt.is_some_and(|receipt| {
        receipt.session_id != session.session_id || receipt.surface != session.surface
    }) {
        return Err(Error::InvalidInput(
            "receipt parent must match resolved session and surface",
        ));
    }
    if (source == SessionSource::Plugin) != request.receipt.is_some() {
        return Err(Error::InvalidInput(
            "plugin batches require receipt facts; other sources cannot supply them",
        ));
    }
    if request.cursor.is_some_and(|cursor| cursor.source != source) {
        return Err(Error::InvalidInput(
            "cursor source must match ingestion source",
        ));
    }
    let records = request
        .records
        .iter()
        .map(|record| record.canonical.clone())
        .collect::<Vec<_>>();
    let identities = request
        .records
        .iter()
        .map(|record| identity(record, request.observed_at))
        .collect::<Vec<_>>();
    let mut projections = BTreeMap::<String, Projection>::new();
    let mut source_masks = BTreeMap::<String, i64>::new();
    for (record, identity) in records.iter().zip(&identities) {
        let Some(uuid) = record
            .uuid
            .as_deref()
            .filter(|uuid| !uuid.trim().is_empty())
        else {
            continue;
        };
        let projection = Projection::from_canonical(&session.session_id, record, identity)?;
        *source_masks.entry(uuid.to_owned()).or_default() |= projection.field_mask();
        if request.receipt.is_some() {
            if let Some(previous) = projections.get_mut(uuid) {
                previous.merge_known(&projection)?;
            } else {
                projections.insert(uuid.to_owned(), projection);
            }
        }
    }
    // A retry is encoded under the digest version its receipt was sealed with,
    // so an unchanged resubmission of a receipt an older build sealed still
    // matches exactly; a new receipt uses the current version.
    let version = match request.receipt {
        Some(receipt) => sealed_version(store, &receipt.receipt_id)?,
        None => SCHEMA_VERSION,
    };
    let coverage = projections
        .iter()
        .map(|(uuid, projection)| {
            covered_as(uuid, projection, version).unwrap_or_else(|| covered(uuid, projection))
        })
        .collect::<Vec<_>>();
    let record_sources = source_masks
        .into_iter()
        .map(|(uuid, field_presence)| RecordSourceObservation {
            uuid,
            source,
            field_presence,
            conflict_flags: 0,
        })
        .collect::<Vec<_>>();
    let session_sources = [SessionSourceObservation {
        session_id: session.session_id.clone(),
        source,
        first_seen_at: request.observed_at,
        last_seen_at: request.observed_at,
    }];
    let confirmed_iteration_usage = request
        .records
        .iter()
        .map(|r| r.native.iteration_usage_confirmed)
        .collect::<Vec<_>>();
    // Derive structural events here, against the session identity this batch
    // resolved, so an event can never be attributed to a session the records
    // were not written to. Their IDs are the native records' own UUIDs.
    let mut tool_events = Vec::new();
    for record in request.records {
        if let Some(event) = tool_use::command_event(record, &session.session_id, source) {
            tool_events.push(event);
        }
    }
    for summary in request.hook_summaries {
        let event = tool_use::hook_event(summary, &session.session_id, source).ok_or(
            Error::InvalidInput("structural tool event requires a stable source event ID"),
        )?;
        tool_events.push(event);
    }
    let pr_links = request
        .pr_witnesses
        .iter()
        .map(|witness| pr_observation(&session, witness))
        .collect::<Result<Vec<_>>>()?;
    let task_notifications = request
        .records
        .iter()
        .map(|record| record.task_notification)
        .collect::<Vec<_>>();
    let tool_sent = request
        .records
        .iter()
        .map(|record| record.tool_sent)
        .collect::<Vec<_>>();
    let withheld_previews = request
        .records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            record.human_adjustment.is_some() || adjusted.get(index).copied().unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let mut batch = IngestBatch::new(&session, &records, request.keep_content);
    if task_notifications.contains(&true) {
        batch.task_notifications = &task_notifications;
    }
    if tool_sent.iter().any(Option::is_some) {
        batch.tool_sent = &tool_sent;
    }
    if withheld_previews.contains(&true) {
        batch.withheld_previews = &withheld_previews;
    }
    batch.tool_events = &tool_events;
    batch.pr_links = &pr_links;
    batch.identities = &identities;
    batch.native_history = source == SessionSource::Transcript
        && session.host == Host::Claude
        && request.discovery.is_some();
    batch.native_codex = source == SessionSource::ReadersCli
        && session.host == Host::Codex
        && request.discovery.is_some();
    if batch.native_history {
        batch.confirmed_iteration_usage = &confirmed_iteration_usage;
    }
    batch.namespace = request.namespace;
    batch.session_sources = &session_sources;
    batch.record_sources = &record_sources;
    batch.receipt = request.receipt.map(|receipt| SubmittedReceipt {
        receipt,
        coverage: &coverage,
    });
    batch.receipt_replay = ReceiptReplay::MatchExact;
    batch.evidence_policy = EvidencePolicy::AcceptedOnly;
    batch.cursor = request.cursor;
    batch.discovery = request.discovery;
    batch.checkpoint = request.checkpoint;
    batch.injected_context = proofs;
    let saved = store.apply_ingest_batch(&batch)?;
    // No code above this point constructs an acknowledgement or emitted event.
    let mut accepted = BTreeMap::<&str, (bool, bool)>::new();
    let mut rejected = BTreeSet::new();
    let mut dropped_reasons = Vec::new();
    let mut missing = 0;
    for row in &saved.records {
        if row.disposition.is_accepted() {
            let counts = accepted.entry(row.uuid.as_deref().unwrap()).or_default();
            counts.0 |= row.disposition == RecordDisposition::Inserted;
            counts.1 |= row.disposition == RecordDisposition::Enriched;
        } else {
            dropped_reasons.push((row.input_index, row.disposition));
            if let Some(uuid) = row.uuid.as_deref() {
                rejected.insert(uuid);
            } else {
                missing += 1;
            }
        }
    }
    let records_new = accepted.values().filter(|(new, _)| *new).count();
    let records_enriched = accepted
        .values()
        .filter(|(new, enriched)| !*new && *enriched)
        .count();
    let records_dropped = missing
        + rejected
            .iter()
            .filter(|uuid| !accepted.contains_key(**uuid))
            .count();
    let ack_through = if saved.receipt_committed {
        saved
            .records
            .iter()
            .rev()
            .find(|row| {
                row.disposition.is_accepted() && !rejected.contains(row.uuid.as_deref().unwrap())
            })
            .and_then(|row| row.uuid.clone())
    } else {
        None
    };
    let invalidate = saved.session_changed
        || records_new > 0
        || records_enriched > 0
        || saved
            .records
            .iter()
            .any(|row| row.conflict_fields != 0 || row.stored_has_conflict == Some(true));
    let mut events = Vec::new();
    if source != SessionSource::Plugin
        || saved
            .records
            .iter()
            .any(|row| row.disposition.is_accepted())
    {
        events.push(ChangeEvent {
            conversation_id: session.session_id.clone(),
            source,
            surface: saved.session_surface,
            records_new,
            records_enriched,
            invalidate_measurements: invalidate || saved.receipt_committed,
            invalidate_cost: invalidate,
            backfill_position: request
                .cursor
                .filter(|_| source != SessionSource::Plugin)
                .map(|cursor| cursor.position),
        });
    }
    events.extend(saved.affected_owners.into_iter().map(|owner| ChangeEvent {
        conversation_id: owner.session_id,
        source,
        surface: owner.surface,
        records_new: 0,
        records_enriched: 0,
        invalidate_measurements: true,
        invalidate_cost: true,
        backfill_position: None,
    }));
    Ok((
        BatchOutcome {
            conversation_id: session.session_id,
            records_new,
            records_enriched,
            records_dropped,
            ack_through,
            dropped_reasons,
            events,
        },
        saved.injected_context,
    ))
}

/// An exact link from one witness. Its pull request must reconcile to one
/// canonical identity from the line's URL, repository and number; its own
/// timestamp is the only event time, never the scan's `observed_at`.
fn pr_observation(session: &SessionMeta, witness: &PrWitness) -> Result<PrLinkObservation> {
    if session.host != Host::Claude || session.source != SessionSource::Transcript {
        return Err(Error::InvalidInput(
            "native PR witnesses require Claude transcript history",
        ));
    }
    let named = witness.named_session.trim();
    if named.is_empty() || named != witness.named_session {
        return Err(Error::InvalidInput("PR witness names no native session"));
    }
    let link = &witness.link;
    let pull_request = PrIdentity::reconcile(
        Some(&link.raw_url),
        Some(&link.raw_repository),
        Some(link.number),
    )?;
    let (_, seen_at) = xt_store::timestamp::parse(
        link.timestamp
            .as_deref()
            .ok_or(Error::InvalidInput("PR witness requires its own timestamp"))?,
    )?;
    Ok(PrLinkObservation {
        // A Claude session's canonical ID is its native ID.
        session_id: witness.named_session.clone(),
        pull_request,
        confidence: PrConfidence::Exact,
        first_seen_at: seen_at,
        last_seen_at: seen_at,
    })
}

fn agree(target: &mut Option<String>, incoming: Option<&String>) -> Result<()> {
    if let Some(value) = incoming {
        if value.trim().is_empty() {
            return Err(Error::InvalidInput("identity labels cannot be empty"));
        }
        match target {
            Some(old) if old != value => {
                return Err(Error::InvalidInput("native and import identity disagree"));
            }
            None => *target = Some(value.clone()),
            _ => {}
        }
    }
    Ok(())
}

/// Resolve submitted identity without I/O or borrowing facts from stored rows.
pub fn resolve_session(request: &WriteBatch<'_>) -> Result<SessionMeta> {
    let mut context = request.context.clone();
    let mut started = context
        .started_at
        .as_deref()
        .map(xt_store::timestamp::parse)
        .transpose()?;
    for value in [
        &context.conversation_id,
        &context.native_session_id,
        &context.source_platform,
        &context.source_surface,
    ]
    .into_iter()
    .flatten()
    {
        if value.trim().is_empty() {
            return Err(Error::InvalidInput("identity labels cannot be empty"));
        }
    }
    let source = context
        .source
        .ok_or(Error::InvalidInput("ingestion source is required"))?;
    for record in request.records.iter().filter(|record| {
        record
            .canonical
            .uuid
            .as_deref()
            .is_some_and(|uuid| !uuid.trim().is_empty())
    }) {
        for observed in [&record.context, &record.source] {
            agree(
                &mut context.conversation_id,
                observed.conversation_id.as_ref(),
            )?;
            agree(
                &mut context.native_session_id,
                observed.native_session_id.as_ref(),
            )?;
            agree(
                &mut context.source_platform,
                observed.source_platform.as_ref(),
            )?;
            agree(
                &mut context.source_surface,
                observed.source_surface.as_ref(),
            )?;
            if let Some(value) = &observed.started_at {
                let incoming = xt_store::timestamp::parse(value)?;
                if started.as_ref().is_some_and(|old| old != &incoming) {
                    return Err(Error::InvalidInput("native start instants disagree"));
                }
                started.get_or_insert(incoming);
            }
            if observed.source.is_some_and(|value| value != source) {
                return Err(Error::InvalidInput(
                    "record source disagrees with its import",
                ));
            }
        }
        agree(
            &mut context.native_session_id,
            record.native.session_id.as_ref(),
        )?;
        agree(
            &mut context.native_session_id,
            record.canonical.native_session_id.as_ref(),
        )?;
        agree(
            &mut context.source_platform,
            record.canonical.source_platform.as_ref(),
        )?;
        agree(
            &mut context.source_surface,
            record.canonical.source_surface.as_ref(),
        )?;
    }
    // A hook summary is not a record, but it names the same native identity and
    // is reconciled by the same rules before anything it implies is persisted.
    // A summary disagreeing with its file or its records fails the batch, so
    // neither the event nor the checkpoint covering it commits.
    for summary in request.hook_summaries {
        for observed in [&summary.context, &summary.source] {
            agree(
                &mut context.conversation_id,
                observed.conversation_id.as_ref(),
            )?;
            agree(
                &mut context.native_session_id,
                observed.native_session_id.as_ref(),
            )?;
            agree(
                &mut context.source_platform,
                observed.source_platform.as_ref(),
            )?;
            agree(
                &mut context.source_surface,
                observed.source_surface.as_ref(),
            )?;
            if let Some(value) = &observed.started_at {
                let incoming = xt_store::timestamp::parse(value)?;
                if started.as_ref().is_some_and(|old| old != &incoming) {
                    return Err(Error::InvalidInput("native start instants disagree"));
                }
                started.get_or_insert(incoming);
            }
            if observed.source.is_some_and(|value| value != source) {
                return Err(Error::InvalidInput(
                    "record source disagrees with its import",
                ));
            }
        }
        agree(
            &mut context.native_session_id,
            summary.native.session_id.as_ref(),
        )?;
        // The summary's own entrypoint is a surface label like a record's.
        agree(
            &mut context.source_surface,
            summary.native.entrypoint.as_ref(),
        )?;
    }
    // A PR witness is reconciled by the same rules: its line's labels, bound to
    // the file context by the adapter, must agree with the batch, or neither
    // the link nor the checkpoint covering it commits.
    for witness in request.pr_witnesses {
        let link = &witness.link;
        for observed in [&link.context, &link.source] {
            agree(
                &mut context.conversation_id,
                observed.conversation_id.as_ref(),
            )?;
            agree(
                &mut context.native_session_id,
                observed.native_session_id.as_ref(),
            )?;
            agree(
                &mut context.source_platform,
                observed.source_platform.as_ref(),
            )?;
            agree(
                &mut context.source_surface,
                observed.source_surface.as_ref(),
            )?;
            if let Some(value) = &observed.started_at {
                let incoming = xt_store::timestamp::parse(value)?;
                if started.as_ref().is_some_and(|old| old != &incoming) {
                    return Err(Error::InvalidInput("native start instants disagree"));
                }
                started.get_or_insert(incoming);
            }
            if observed.source.is_some_and(|value| value != source) {
                return Err(Error::InvalidInput(
                    "record source disagrees with its import",
                ));
            }
        }
        agree(
            &mut context.native_session_id,
            link.native.session_id.as_ref(),
        )?;
        agree(&mut context.source_surface, link.native.entrypoint.as_ref())?;
    }
    let host = context
        .source_platform
        .as_deref()
        .map(Host::from_platform)
        .unwrap_or(Host::Other);
    if request
        .declared_host
        .is_some_and(|declared| context.source_platform.is_some() && declared != host)
    {
        return Err(Error::InvalidInput(
            "declared host and raw platform disagree",
        ));
    }
    let host = request.declared_host.unwrap_or(host);
    let canonical = match (&context.conversation_id, &context.native_session_id) {
        (Some(id), _) => id.clone(),
        (None, Some(native)) => match host {
            Host::Claude => native.clone(),
            Host::Codex => format!("codex-{native}"),
            Host::Cursor => format!("cursor-{native}"),
            Host::Other => {
                return Err(Error::InvalidInput(
                    "unknown host requires a canonical conversation ID",
                ));
            }
        },
        _ => {
            return Err(Error::InvalidInput(
                "canonical conversation identity is missing",
            ));
        }
    };
    if let Some(native) = &context.native_session_id {
        let expected = match host {
            Host::Claude => Some(native.clone()),
            Host::Codex => Some(format!("codex-{native}")),
            Host::Cursor => Some(format!("cursor-{native}")),
            Host::Other => None,
        };
        if expected.is_some_and(|id| id != canonical) {
            return Err(Error::InvalidInput(
                "canonical and native session IDs disagree",
            ));
        }
    }
    let started_at_ms = started.map(|(_, millis)| millis);
    Ok(SessionMeta {
        session_id: canonical,
        host,
        source_platform: context.source_platform,
        source,
        cwd: request.cwd.map(str::to_owned),
        git_branch: request.git_branch.map(str::to_owned),
        title: request.title.map(str::to_owned),
        surface: context.source_surface,
        surface_evidence: None,
        native_session_id: context.native_session_id,
        started_at_ms,
    })
}
