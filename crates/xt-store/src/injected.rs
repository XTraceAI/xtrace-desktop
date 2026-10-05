//! Source proofs that Codex itself injected a saved user input.
//!
//! The native reader, reading one exactly selected Codex session, can declare
//! that a canonical user record it emitted came from a native item Codex
//! injected: the body it adds when a skill is selected, beside (not instead
//! of) the person's own request. The declaration rests on native content kinds
//! and identifiers, never on text, and carries only identities. A strict
//! importer hands it to [`crate::batch::IngestBatch::injected_context`],
//! aligned with the record it names.
//!
//! A proof is only ever evidence for the ingestion that first stores its
//! record. It is kept only when that exact input inserted the record in the
//! same transaction, alone and unconflicted, as an eligible human-classified
//! user text input of Codex reader history. A record the index already held
//! (a replay, an enrichment, an identical retry, another session's copy) is
//! left exactly as it was: a later read is not proof of how an earlier row was
//! built. There is no API that attaches a proof to an existing record.
//!
//! A proof whose shape or binding is wrong rejects the whole batch. One that
//! is well formed but finds its input ineligible abstains, and the batch's
//! records commit as they would without it. Raw classification, the record,
//! its usage and its tools are never changed; the shared record projection
//! reads the proof as a durable override of that one input's human
//! eligibility, exactly as it reads a confirmation.

use crate::{
    Error, Host, Result, SessionSource,
    batch::{IngestBatch, RecordDisposition, RecordOutcome},
    confirmation::{self, Abstention, canonical_uuid},
    model::text_enum,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

text_enum!(OriginContract { CodexOriginEvidence => "memhub.codex.origin_evidence" });

// The closed set of injected context a proof may name.
// `codex_selected_skill_instructions`: a user message item whose every native
// content block Codex declared as the instructions of a selected skill.
text_enum!(InjectedContextKind {
    CodexSelectedSkillInstructions => "codex_selected_skill_instructions"
});

text_enum!(SegmentHistory { Flat => "flat", Paginated => "paginated" });

/// The only evidence version this build accepts.
pub const ORIGIN_EVIDENCE_VERSION: u32 = 1;

/// The identities that prove one canonical Codex record was converted from a
/// native item Codex injected. None is text, a text digest, a path or a title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InjectedContextProof {
    pub contract: OriginContract,
    pub version: u32,
    pub kind: InjectedContextKind,
    /// The session's native identity; the batch's session must be
    /// `codex-` followed by it and state it as its own.
    pub native_session_id: String,
    pub history: SegmentHistory,
    /// None for a flat rollout; the rollout's UUID in a paginated group.
    pub rollout_id: Option<String>,
    /// Zero-based index of the native row among that rollout's rows.
    pub row_index: u32,
    /// The native row's own ordinal, when it has one.
    pub row_ordinal: Option<u32>,
    /// The native message item ID.
    pub item_id: String,
    /// The turn open at the native row.
    pub turn_id: String,
    /// The canonical record converted from the native row. It must be the
    /// UUID of the input this proof is aligned with.
    pub record_uuid: String,
}

/// Why a well-formed proof was not kept. None is an error: the record commits
/// as ingestion stored it, and remains a human-classified input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectedContextAbstention {
    /// This input did not insert its record: the UUID was already indexed, or
    /// the input was rejected. A proof never joins an existing row.
    NotInserted,
    /// The record's observations disagree, now or in this very input.
    ConflictedRecord,
    /// The input is not one the confirmation rules could correct: another
    /// session identity, host or source, not a user text input, or not
    /// human-classified.
    Ineligible(Abstention),
    /// Another proof already names this native item or native row, or an
    /// existing confirmation holds this input.
    ConflictingProof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectedContextDisposition {
    /// Stored in this batch's transaction with its record.
    Recorded,
    Abstained(InjectedContextAbstention),
}

/// One result per supplied proof, in input order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InjectedContextOutcome {
    pub input_index: usize,
    pub disposition: InjectedContextDisposition,
}

impl crate::Store {
    /// The stored injected context proofs of one session, ordered by record
    /// UUID. Read-only: a proof is written only with its record's insertion.
    pub fn injected_context_proofs(&self, session_id: &str) -> Result<Vec<InjectedContextProof>> {
        Ok(self
            .connection
            .prepare(
                "SELECT evidence_contract,evidence_version,evidence_kind,native_session_id,
                     segment_history,rollout_id,row_index,row_ordinal,item_id,turn_id,record_uuid
                 FROM injected_context_inputs WHERE session_id=?1 ORDER BY record_uuid",
            )?
            .query_map([session_id], |row| {
                Ok(InjectedContextProof {
                    contract: row.get(0)?,
                    version: row.get(1)?,
                    kind: row.get(2)?,
                    native_session_id: row.get(3)?,
                    history: row.get(4)?,
                    rollout_id: row.get(5)?,
                    row_index: row.get(6)?,
                    row_ordinal: row.get(7)?,
                    item_id: row.get(8)?,
                    turn_id: row.get(9)?,
                    record_uuid: row.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

fn item_id(value: &str) -> bool {
    value.strip_prefix("msg_").is_some_and(|rest| {
        canonical_uuid(rest)
            || (rest.len() == 50 && rest.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
    })
}

/// Shape, binding and ambiguity checks that need no stored state. Any failure
/// rejects the whole batch before anything is written.
pub(crate) fn validate(batch: &IngestBatch<'_>) -> Result<()> {
    if batch.injected_context.is_empty() {
        return Ok(());
    }
    if !batch.native_codex {
        return Err(Error::InvalidInput(
            "injected context proofs require native Codex reader history",
        ));
    }
    if batch.injected_context.len() != batch.records.len() {
        return Err(Error::InvalidInput(
            "injected context proofs must align with batch inputs",
        ));
    }
    let shaped = |condition: bool| {
        if condition {
            Ok(())
        } else {
            Err(Error::InvalidInput(
                "injected context proof requires closed canonical identities",
            ))
        }
    };
    let mut claimed = BTreeSet::new();
    let mut items = BTreeSet::new();
    let mut rows = BTreeSet::new();
    for (proof, input) in batch.injected_context.iter().zip(batch.records) {
        let Some(proof) = proof else { continue };
        shaped(
            proof.version == ORIGIN_EVIDENCE_VERSION
                && canonical_uuid(&proof.native_session_id)
                && canonical_uuid(&proof.turn_id)
                && canonical_uuid(&proof.record_uuid)
                && item_id(&proof.item_id)
                && match proof.history {
                    SegmentHistory::Flat => proof.rollout_id.is_none(),
                    SegmentHistory::Paginated => {
                        proof.rollout_id.as_deref().is_some_and(canonical_uuid)
                    }
                },
        )?;
        let native = proof.native_session_id.as_str();
        if batch.session.session_id != format!("codex-{native}")
            || batch.session.native_session_id.as_deref() != Some(native)
            || input.uuid.as_deref() != Some(proof.record_uuid.as_str())
        {
            return Err(Error::InvalidInput(
                "injected context proof must name its own input and session",
            ));
        }
        if !claimed.insert(proof.record_uuid.as_str())
            || !items.insert(proof.item_id.as_str())
            || !rows.insert((proof.rollout_id.as_deref(), proof.row_index))
        {
            return Err(Error::InvalidInput(
                "injected context proofs name one input, item or row twice",
            ));
        }
    }
    // A claimed UUID occurs once among the inputs, so this input's outcome is
    // the only thing the batch says about that record.
    if batch
        .records
        .iter()
        .filter_map(|record| record.uuid.as_deref())
        .filter(|uuid| claimed.contains(uuid))
        .count()
        != claimed.len()
    {
        return Err(Error::InvalidInput(
            "an injected context proof's record must occur once in its batch",
        ));
    }
    Ok(())
}

/// Store each validated proof whose own input inserted an eligible record in
/// this transaction. Called once, after the records are written and before
/// the batch commits.
pub(crate) fn apply(
    connection: &Connection,
    batch: &IngestBatch<'_>,
    outcomes: &[RecordOutcome],
) -> Result<Vec<InjectedContextOutcome>> {
    let Some(discovery) = batch.discovery else {
        return Ok(Vec::new());
    };
    let mut results = Vec::new();
    for (index, proof) in batch.injected_context.iter().enumerate() {
        let Some(proof) = proof else { continue };
        let outcome = outcomes
            .get(index)
            .filter(|outcome| outcome.input_index == index)
            .ok_or(Error::IncompatibleSchema)?;
        let disposition = match abstention(connection, proof, outcome)? {
            Some(reason) => InjectedContextDisposition::Abstained(reason),
            None => {
                connection.execute(
                    "INSERT INTO injected_context_inputs(record_uuid,session_id,native_session_id,
                         evidence_contract,evidence_version,evidence_kind,segment_history,
                         rollout_id,row_index,row_ordinal,item_id,turn_id,observed_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                    params![
                        proof.record_uuid,
                        batch.session.session_id,
                        proof.native_session_id,
                        proof.contract,
                        proof.version,
                        proof.kind,
                        proof.history,
                        proof.rollout_id,
                        proof.row_index,
                        proof.row_ordinal,
                        proof.item_id,
                        proof.turn_id,
                        discovery.last_observed_at,
                    ],
                )?;
                InjectedContextDisposition::Recorded
            }
        };
        results.push(InjectedContextOutcome {
            input_index: index,
            disposition,
        });
    }
    Ok(results)
}

fn abstention(
    connection: &Connection,
    proof: &InjectedContextProof,
    outcome: &RecordOutcome,
) -> Result<Option<InjectedContextAbstention>> {
    use InjectedContextAbstention as Why;
    if outcome.disposition != RecordDisposition::Inserted
        || outcome.uuid.as_deref() != Some(proof.record_uuid.as_str())
    {
        return Ok(Some(Why::NotInserted));
    }
    if outcome.conflict_fields != 0 || outcome.stored_has_conflict != Some(false) {
        return Ok(Some(Why::ConflictedRecord));
    }
    // The stored row and its session as this transaction now holds them: a
    // session first stored from another source, or with another native
    // identity, keeps what it had.
    let target =
        confirmation::target(connection, &proof.record_uuid)?.ok_or(Error::IncompatibleSchema)?;
    if target.host != Host::Codex || target.source != SessionSource::ReadersCli {
        return Ok(Some(Why::Ineligible(Abstention::NotCodexReaderHistory)));
    }
    if let Some(reason) = confirmation::input_abstention(&target, &proof.native_session_id) {
        return Ok(Some(Why::Ineligible(reason)));
    }
    if target.has_conflict {
        return Ok(Some(Why::ConflictedRecord));
    }
    let held: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM confirmed_automated_inputs WHERE record_uuid=?1)
             OR EXISTS(SELECT 1 FROM guardian_turn_inputs WHERE record_uuid=?1)
             OR EXISTS(SELECT 1 FROM injected_context_inputs WHERE record_uuid=?1
                 OR (native_session_id=?2 AND item_id=?3)
                 OR (native_session_id=?2 AND coalesce(rollout_id,'')=coalesce(?4,'')
                     AND row_index=?5))",
        params![
            proof.record_uuid,
            proof.native_session_id,
            proof.item_id,
            proof.rollout_id,
            proof.row_index
        ],
        |row| row.get(0),
    )?;
    Ok(held.then_some(Why::ConflictingProof))
}
