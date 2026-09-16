//! Synchronous ingestion composition. Returned acknowledgements and events exist
//! only after the single store transaction commits; adapters publish them later.

use crate::canonical::{ParsedRecord, SourceContext};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use xt_store::{
    Error, Host, Result, SessionMeta, SessionSource, Store, StoredRecord,
    batch::{
        EvidencePolicy, IngestBatch, ReceiptReplay, RecordDisposition, SourceCursor,
        SubmittedReceipt,
    },
    ingest::{CaptureReceipt, RecordCoverage, RecordSourceObservation, SessionSourceObservation},
    measurement::{Projection, SCHEMA_VERSION},
    model::RecordIdentity,
};

pub const MAX_BATCH_RECORDS: usize = 2_000;

pub struct WriteBatch<'a> {
    pub context: &'a SourceContext,
    /// A known adapter host need not manufacture an absent raw platform label.
    pub declared_host: Option<Host>,
    pub records: &'a [ParsedRecord],
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
pub fn matches_current(coverage: &RecordCoverage, record: &StoredRecord) -> Result<bool> {
    let projection = Projection::from_stored(record)?;
    Ok(!record.has_conflict && coverage == &covered(&record.uuid, &projection))
}

fn covered(uuid: &str, projection: &Projection) -> RecordCoverage {
    RecordCoverage {
        record_uuid: uuid.to_owned(),
        metric_field_mask: projection.field_mask(),
        measurement_revision: format!("{:x}", Sha256::digest(projection.canonical_bytes())),
        digest_schema_version: SCHEMA_VERSION,
    }
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
    if request.records.len() > MAX_BATCH_RECORDS {
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
    let coverage = projections
        .iter()
        .map(|(uuid, projection)| covered(uuid, projection))
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
    let mut batch = IngestBatch::new(&session, &records, request.keep_content);
    batch.identities = &identities;
    batch.native_history = source == SessionSource::Transcript
        && session.host == Host::Claude
        && request.discovery.is_some();
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
    Ok(BatchOutcome {
        conversation_id: session.session_id,
        records_new,
        records_enriched,
        records_dropped,
        ack_through,
        dropped_reasons,
        events,
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
