//! Structural confirmations that another agent, not a person, submitted a
//! saved user input.
//!
//! A caller that has proven the relationship — a parent agent's tool call, its
//! paired successful result naming the target session, and the target's saved
//! input matching what the call submitted — hands the resulting identities to
//! [`Store::apply_automated_input_confirmations`]. The comparison that proved it
//! happens elsewhere and in memory; nothing here accepts or keeps a prompt, a
//! command, an output body or a digest of any of them.
//!
//! A confirmation binds exactly one existing input in its owning session and is
//! checked against that session's native identity. It never changes the raw
//! classification ingestion stored: the shared record projection reads it as a
//! durable override of that one input's human eligibility, so a later replay
//! that lacks the evidence cannot restore the input to human, and the record's
//! UUID, session, order and work stay exactly as they were.
//!
//! Anything the store cannot tie to one eligible input abstains instead of
//! guessing: a missing or differently owned target, an unknown or different
//! native session, an input that is not a human-classified user text record, and
//! any disagreement with an earlier confirmation or inside the same batch.
//!
//! [`Store::apply_guardian_turn_confirmations`] is the sibling for a different
//! relationship: a Codex Guardian reviewer's single input in one of its turns,
//! dispatched by an exact turn of its parent thread. That parent link is a
//! turn, not a tool call, so it is kept in its own table and never presented as
//! a dispatching call. It is a historical correction path for inputs already
//! indexed: a private caller that has checked the saved rollout and its logs
//! supplies the identities, and the store accepts them as structure without
//! claiming to have verified those sources. Both tables feed one effective
//! classification, and each input can hold only one confirmation.
//!
//! A third source, [`crate::injected`], proves Codex injected an input. It is
//! accepted only with the record's first insertion, never through these APIs,
//! and an input it holds is refused here as already confirmed.

use crate::{
    Error, Host, Result, SessionSource, Store,
    batch::AffectedSession,
    model::{RecordType, text_enum},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Proofs accepted by one call. A caller with more splits them into batches.
pub const MAX_CONFIRMATIONS: usize = 1_000;
/// Longest accepted structural identifier, in bytes.
const MAX_IDENTIFIER: usize = 256;

// The closed set of evidence a confirmation may rest on. `agent_dispatch`: a
// parent agent's tool call launched or resumed the target session, its paired
// successful result returned that session's identity, and the target's saved
// input is exactly the input the call submitted.
text_enum!(EvidenceKind { AgentDispatch => "agent_dispatch" });

// The closed set of evidence a Guardian turn confirmation may rest on.
// `guardian_turn_dispatch`: the Guardian reviewer's turn was started by an exact
// turn of its parent thread, and the target is that turn's single user input.
text_enum!(GuardianEvidenceKind { GuardianTurnDispatch => "guardian_turn_dispatch" });

/// The identities that prove one saved input was submitted by another agent.
/// Every field is structural; none is transcript content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomatedInputProof {
    /// The saved input's canonical record UUID.
    pub record_uuid: String,
    /// The canonical session that owns that record.
    pub session_id: String,
    /// The native session identity the dispatch result returned. It must equal
    /// the indexed session's own native identity.
    pub native_session_id: String,
    pub parent_host: Host,
    /// The dispatching agent's canonical session. It need not be indexed.
    pub parent_session_id: String,
    /// The dispatching tool call's own identifier.
    pub parent_tool_call_id: String,
    /// Zero-based position of the dispatch operation inside that call, for a
    /// call that ran several commands. Its result is the one at this position.
    pub parent_operation_index: u32,
    /// The paired successful result's own identifier, when it has one.
    pub parent_result_id: Option<String>,
    pub evidence_kind: EvidenceKind,
    /// Version of the matcher that established the relationship.
    pub matcher_version: u32,
}

/// The identities that prove one saved Codex input is the input a parent
/// thread's turn dispatched to a Guardian reviewer turn. Every field is a
/// canonical lowercase UUID, except the owning session, which is `codex-`
/// followed by its native identity; none is transcript or log content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuardianTurnProof {
    /// The saved input's canonical record UUID.
    pub record_uuid: String,
    /// The canonical Codex session that owns that record.
    pub session_id: String,
    /// The Guardian reviewer thread's native identity. It must equal the
    /// indexed session's own native identity.
    pub native_session_id: String,
    /// The reviewer's own turn whose single user input is the record.
    pub turn_id: String,
    /// The parent thread that started the reviewer turn. It need not be
    /// indexed, and it is never the reviewer itself.
    pub parent_native_session_id: String,
    /// The parent thread's turn that started the reviewer turn.
    pub parent_turn_id: String,
    pub evidence_kind: GuardianEvidenceKind,
    /// Version of the matcher that established the relationship.
    pub matcher_version: u32,
}

/// Why one proof changed nothing. None of these is an error: the store simply
/// does not know enough to confirm that input, and leaves it as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abstention {
    /// No record has this UUID.
    MissingTarget,
    /// The record is owned by another session, including a session that holds
    /// it only as a copied native context.
    SessionMismatch,
    /// A Guardian turn proof names a session that is not Codex history read by
    /// the native reader.
    NotCodexReaderHistory,
    /// The owning session's native identity is unknown or differs.
    NativeIdentityMismatch,
    /// Not a user input: another record type or role, meta or sidechain
    /// context, a tool-result carrier, or a judge session.
    NotUserInput,
    /// The stored classification is not human, or is unknown. A confirmation
    /// corrects a human-classified input; it never decides an unknown one.
    NotHumanClassified,
    /// A Guardian turn proof names a record whose saved copies disagree.
    ConflictedRecord,
    /// A different confirmation, in either confirmation table, or an injected
    /// context proof already holds this input, or this proof's parent
    /// operation or Guardian turn already confirms a different input.
    ConflictingConfirmation,
    /// The same batch names this input, or this parent operation or Guardian
    /// turn, again with different facts. Neither is applied.
    AmbiguousInBatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmationDisposition {
    /// Newly stored.
    Confirmed,
    /// This exact proof was already stored; nothing changed.
    AlreadyConfirmed,
    Abstained(Abstention),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmationReport {
    /// One disposition per supplied proof, in input order.
    pub dispositions: Vec<ConfirmationDisposition>,
    /// Sessions whose effective classification changed in this call; their
    /// measurements must be invalidated after the commit.
    pub affected_sessions: Vec<AffectedSession>,
}

impl Store {
    /// Apply structural confirmations in one immediate transaction.
    ///
    /// Malformed identifiers or too many proofs reject the whole call before
    /// anything is written. Otherwise every proof receives a disposition, and
    /// the confirmations that were stored commit together.
    pub fn apply_automated_input_confirmations(
        &mut self,
        proofs: &[AutomatedInputProof],
        confirmed_at: i64,
    ) -> Result<ConfirmationReport> {
        if proofs.len() > MAX_CONFIRMATIONS {
            return Err(Error::InvalidInput(
                "too many automated input confirmations in one call",
            ));
        }
        for proof in proofs {
            validate(proof)?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ambiguous = ambiguous(proofs);
        let mut dispositions = Vec::with_capacity(proofs.len());
        let mut affected = BTreeMap::<String, AffectedSession>::new();
        for (index, proof) in proofs.iter().enumerate() {
            let disposition = if ambiguous.contains(&index) {
                ConfirmationDisposition::Abstained(Abstention::AmbiguousInBatch)
            } else {
                apply(&transaction, proof, confirmed_at, &mut affected)?
            };
            dispositions.push(disposition);
        }
        transaction.commit()?;
        Ok(ConfirmationReport {
            dispositions,
            affected_sessions: affected.into_values().collect(),
        })
    }

    /// Apply Guardian turn confirmations in one immediate transaction, with the
    /// same all-or-nothing validation and per-proof dispositions as
    /// [`Store::apply_automated_input_confirmations`].
    ///
    /// Only a human-classified, unconflicted user input of indexed Codex reader
    /// history is eligible, one input per Guardian turn. Other inputs of the
    /// same reviewer session are never touched: a proof confirms one record.
    pub fn apply_guardian_turn_confirmations(
        &mut self,
        proofs: &[GuardianTurnProof],
        confirmed_at: i64,
    ) -> Result<ConfirmationReport> {
        if proofs.len() > MAX_CONFIRMATIONS {
            return Err(Error::InvalidInput(
                "too many guardian turn confirmations in one call",
            ));
        }
        for proof in proofs {
            validate_guardian(proof)?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ambiguous = ambiguous_guardian(proofs);
        let mut dispositions = Vec::with_capacity(proofs.len());
        let mut affected = BTreeMap::<String, AffectedSession>::new();
        for (index, proof) in proofs.iter().enumerate() {
            let disposition = if ambiguous.contains(&index) {
                ConfirmationDisposition::Abstained(Abstention::AmbiguousInBatch)
            } else {
                apply_guardian(&transaction, proof, confirmed_at, &mut affected)?
            };
            dispositions.push(disposition);
        }
        transaction.commit()?;
        Ok(ConfirmationReport {
            dispositions,
            affected_sessions: affected.into_values().collect(),
        })
    }

    /// The stored Guardian turn confirmations of one session, ordered by record
    /// UUID.
    pub fn guardian_turn_confirmations(&self, session_id: &str) -> Result<Vec<GuardianTurnProof>> {
        Ok(self
            .connection
            .prepare(&format!(
                "SELECT {GUARDIAN_FIELDS} FROM guardian_turn_inputs
                 WHERE session_id=?1 ORDER BY record_uuid"
            ))?
            .query_map([session_id], guardian_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The stored confirmations of one session, ordered by record UUID.
    pub fn automated_input_confirmations(
        &self,
        session_id: &str,
    ) -> Result<Vec<AutomatedInputProof>> {
        Ok(self
            .connection
            .prepare(&format!(
                "SELECT {PROOF_FIELDS} FROM confirmed_automated_inputs
                 WHERE session_id=?1 ORDER BY record_uuid"
            ))?
            .query_map([session_id], proof_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

const PROOF_FIELDS: &str = "record_uuid,session_id,native_session_id,parent_host,parent_session_id,
    parent_tool_call_id,parent_operation_index,parent_result_id,evidence_kind,matcher_version";

fn proof_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutomatedInputProof> {
    Ok(AutomatedInputProof {
        record_uuid: row.get(0)?,
        session_id: row.get(1)?,
        native_session_id: row.get(2)?,
        parent_host: row.get(3)?,
        parent_session_id: row.get(4)?,
        parent_tool_call_id: row.get(5)?,
        parent_operation_index: row.get(6)?,
        parent_result_id: row.get(7)?,
        evidence_kind: row.get(8)?,
        matcher_version: row.get(9)?,
    })
}

const GUARDIAN_FIELDS: &str = "record_uuid,session_id,native_session_id,turn_id,
    parent_native_session_id,parent_turn_id,evidence_kind,matcher_version";

fn guardian_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GuardianTurnProof> {
    Ok(GuardianTurnProof {
        record_uuid: row.get(0)?,
        session_id: row.get(1)?,
        native_session_id: row.get(2)?,
        turn_id: row.get(3)?,
        parent_native_session_id: row.get(4)?,
        parent_turn_id: row.get(5)?,
        evidence_kind: row.get(6)?,
        matcher_version: row.get(7)?,
    })
}

fn identifier(value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > MAX_IDENTIFIER
        || value.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput(
            "automated input confirmation requires bounded structural identifiers",
        ));
    }
    Ok(())
}

fn validate(proof: &AutomatedInputProof) -> Result<()> {
    for value in [
        &proof.record_uuid,
        &proof.session_id,
        &proof.native_session_id,
        &proof.parent_session_id,
        &proof.parent_tool_call_id,
    ]
    .into_iter()
    .chain(proof.parent_result_id.as_ref())
    {
        identifier(value)?;
    }
    if proof.matcher_version == 0 {
        return Err(Error::InvalidInput(
            "automated input confirmation requires a matcher version",
        ));
    }
    Ok(())
}

/// A lowercase hyphenated UUID: the only shape Codex native identities, turn
/// identities and the reader's record UUIDs take.
pub(crate) fn canonical_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => matches!(byte, b'0'..=b'9' | b'a'..=b'f'),
        })
}

fn validate_guardian(proof: &GuardianTurnProof) -> Result<()> {
    if ![
        &proof.record_uuid,
        &proof.native_session_id,
        &proof.turn_id,
        &proof.parent_native_session_id,
        &proof.parent_turn_id,
    ]
    .into_iter()
    .all(|value| canonical_uuid(value))
        || proof.session_id != format!("codex-{}", proof.native_session_id)
    {
        return Err(Error::InvalidInput(
            "guardian turn confirmation requires canonical Codex identities",
        ));
    }
    if proof.parent_native_session_id == proof.native_session_id
        || proof.parent_turn_id == proof.turn_id
    {
        return Err(Error::InvalidInput(
            "guardian turn confirmation requires a parent distinct from the reviewer",
        ));
    }
    if proof.matcher_version == 0 {
        return Err(Error::InvalidInput(
            "guardian turn confirmation requires a matcher version",
        ));
    }
    Ok(())
}

/// Indexes of Guardian proofs the batch contradicts: the same input or the
/// same reviewer turn named again with any different fact.
fn ambiguous_guardian(proofs: &[GuardianTurnProof]) -> BTreeSet<usize> {
    let mut by_target = BTreeMap::<&str, Vec<usize>>::new();
    let mut by_turn = BTreeMap::<(&str, &str), Vec<usize>>::new();
    for (index, proof) in proofs.iter().enumerate() {
        by_target.entry(&proof.record_uuid).or_default().push(index);
        by_turn
            .entry((&proof.native_session_id, &proof.turn_id))
            .or_default()
            .push(index);
    }
    by_target
        .into_values()
        .chain(by_turn.into_values())
        .filter(|group| group.iter().any(|&i| proofs[i] != proofs[group[0]]))
        .flatten()
        .collect()
}

type ParentOperation<'a> = (&'static str, &'a str, &'a str, u32);

fn parent(proof: &AutomatedInputProof) -> ParentOperation<'_> {
    (
        proof.parent_host.as_str(),
        &proof.parent_session_id,
        &proof.parent_tool_call_id,
        proof.parent_operation_index,
    )
}

/// Indexes of proofs the batch contradicts: the same input or the same parent
/// operation named again with any different fact. Exact repeats are not
/// contradictions; the first applies and the rest find it already stored.
fn ambiguous(proofs: &[AutomatedInputProof]) -> BTreeSet<usize> {
    let mut by_target = BTreeMap::<&str, Vec<usize>>::new();
    let mut by_parent = BTreeMap::<ParentOperation<'_>, Vec<usize>>::new();
    for (index, proof) in proofs.iter().enumerate() {
        by_target.entry(&proof.record_uuid).or_default().push(index);
        by_parent.entry(parent(proof)).or_default().push(index);
    }
    by_target
        .into_values()
        .chain(by_parent.into_values())
        .filter(|group| group.iter().any(|&i| proofs[i] != proofs[group[0]]))
        .flatten()
        .collect()
}

fn apply(
    connection: &Connection,
    proof: &AutomatedInputProof,
    confirmed_at: i64,
    affected: &mut BTreeMap<String, AffectedSession>,
) -> Result<ConfirmationDisposition> {
    let abstain = |reason| Ok(ConfirmationDisposition::Abstained(reason));
    let stored = connection
        .query_row(
            &format!("SELECT {PROOF_FIELDS} FROM confirmed_automated_inputs WHERE record_uuid=?1"),
            [&proof.record_uuid],
            proof_from_row,
        )
        .optional()?;
    if let Some(stored) = stored {
        return if &stored == proof {
            Ok(ConfirmationDisposition::AlreadyConfirmed)
        } else {
            abstain(Abstention::ConflictingConfirmation)
        };
    }
    if held_outside_dispatch(connection, &proof.record_uuid)? {
        return abstain(Abstention::ConflictingConfirmation);
    }
    let (host, session, tool_call, operation) = parent(proof);
    let parent_used: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM confirmed_automated_inputs WHERE parent_host=?1
         AND parent_session_id=?2 AND parent_tool_call_id=?3 AND parent_operation_index=?4)",
        params![host, session, tool_call, operation],
        |row| row.get(0),
    )?;
    if parent_used {
        return abstain(Abstention::ConflictingConfirmation);
    }
    let Some(target) = target(connection, &proof.record_uuid)? else {
        return abstain(Abstention::MissingTarget);
    };
    if target.session_id != proof.session_id {
        return abstain(Abstention::SessionMismatch);
    }
    if let Some(reason) = input_abstention(&target, &proof.native_session_id) {
        return abstain(reason);
    }
    connection.execute(
        "INSERT INTO confirmed_automated_inputs(record_uuid,session_id,native_session_id,
             evidence_kind,matcher_version,parent_host,parent_session_id,parent_tool_call_id,
             parent_operation_index,parent_result_id,confirmed_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            proof.record_uuid,
            proof.session_id,
            proof.native_session_id,
            proof.evidence_kind,
            proof.matcher_version,
            proof.parent_host,
            proof.parent_session_id,
            proof.parent_tool_call_id,
            proof.parent_operation_index,
            proof.parent_result_id,
            confirmed_at,
        ],
    )?;
    affected.insert(
        target.session_id.clone(),
        AffectedSession {
            session_id: target.session_id,
            surface: target.surface,
        },
    );
    Ok(ConfirmationDisposition::Confirmed)
}

fn apply_guardian(
    connection: &Connection,
    proof: &GuardianTurnProof,
    confirmed_at: i64,
    affected: &mut BTreeMap<String, AffectedSession>,
) -> Result<ConfirmationDisposition> {
    let abstain = |reason| Ok(ConfirmationDisposition::Abstained(reason));
    let stored = connection
        .query_row(
            &format!("SELECT {GUARDIAN_FIELDS} FROM guardian_turn_inputs WHERE record_uuid=?1"),
            [&proof.record_uuid],
            guardian_from_row,
        )
        .optional()?;
    if let Some(stored) = stored {
        return if &stored == proof {
            Ok(ConfirmationDisposition::AlreadyConfirmed)
        } else {
            abstain(Abstention::ConflictingConfirmation)
        };
    }
    let held: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM confirmed_automated_inputs WHERE record_uuid=?1)
             OR EXISTS(SELECT 1 FROM injected_context_inputs WHERE record_uuid=?1)
             OR EXISTS(SELECT 1 FROM guardian_turn_inputs
                       WHERE native_session_id=?2 AND turn_id=?3)",
        params![proof.record_uuid, proof.native_session_id, proof.turn_id],
        |row| row.get(0),
    )?;
    if held {
        return abstain(Abstention::ConflictingConfirmation);
    }
    let Some(target) = target(connection, &proof.record_uuid)? else {
        return abstain(Abstention::MissingTarget);
    };
    if target.session_id != proof.session_id {
        return abstain(Abstention::SessionMismatch);
    }
    if target.host != Host::Codex || target.source != SessionSource::ReadersCli {
        return abstain(Abstention::NotCodexReaderHistory);
    }
    if let Some(reason) = input_abstention(&target, &proof.native_session_id) {
        return abstain(reason);
    }
    if target.has_conflict {
        return abstain(Abstention::ConflictedRecord);
    }
    connection.execute(
        "INSERT INTO guardian_turn_inputs(record_uuid,session_id,native_session_id,turn_id,
             parent_native_session_id,parent_turn_id,evidence_kind,matcher_version,confirmed_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            proof.record_uuid,
            proof.session_id,
            proof.native_session_id,
            proof.turn_id,
            proof.parent_native_session_id,
            proof.parent_turn_id,
            proof.evidence_kind,
            proof.matcher_version,
            confirmed_at,
        ],
    )?;
    affected.insert(
        target.session_id.clone(),
        AffectedSession {
            session_id: target.session_id,
            surface: target.surface,
        },
    );
    Ok(ConfirmationDisposition::Confirmed)
}

/// Whether a Guardian turn confirmation or an injected context proof holds the
/// input, either of which excludes a tool-call confirmation of it.
fn held_outside_dispatch(connection: &Connection, record_uuid: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM guardian_turn_inputs WHERE record_uuid=?1)
             OR EXISTS(SELECT 1 FROM injected_context_inputs WHERE record_uuid=?1)",
        [record_uuid],
        |row| row.get(0),
    )?)
}

pub(crate) fn target(connection: &Connection, record_uuid: &str) -> Result<Option<Target>> {
    Ok(connection
        .query_row(
            "SELECT r.session_id,r.type,r.role,r.is_meta,r.is_sidechain,r.is_tool_result_carrier,
                 r.is_human,r.has_conflict,s.native_session_id,s.kind,s.surface,s.host,s.source
             FROM records r JOIN sessions s ON s.session_id=r.session_id WHERE r.uuid=?1",
            [record_uuid],
            |row| {
                Ok(Target {
                    session_id: row.get(0)?,
                    record_type: row.get(1)?,
                    role: row.get(2)?,
                    is_meta: row.get(3)?,
                    is_sidechain: row.get(4)?,
                    tool_result_carrier: row.get(5)?,
                    is_human: row.get(6)?,
                    has_conflict: row.get(7)?,
                    native_session_id: row.get(8)?,
                    kind: row.get(9)?,
                    surface: row.get(10)?,
                    host: row.get(11)?,
                    source: row.get(12)?,
                })
            },
        )
        .optional()?)
}

/// Why a target in the proof's session is not an eligible input, if it is not.
pub(crate) fn input_abstention(target: &Target, native_session_id: &str) -> Option<Abstention> {
    if target.native_session_id.as_deref() != Some(native_session_id) {
        return Some(Abstention::NativeIdentityMismatch);
    }
    if target.record_type != RecordType::User
        || target.role.as_deref() != Some("user")
        || target.is_meta
        || target.is_sidechain
        || target.tool_result_carrier == Some(true)
        || target.kind != "user"
    {
        return Some(Abstention::NotUserInput);
    }
    // Unknown content leaves both the classification and the carrier unknown.
    if target.is_human != Some(true) {
        return Some(Abstention::NotHumanClassified);
    }
    if target.tool_result_carrier != Some(false) {
        return Some(Abstention::NotUserInput);
    }
    None
}

pub(crate) struct Target {
    pub(crate) session_id: String,
    record_type: RecordType,
    role: Option<String>,
    is_meta: bool,
    is_sidechain: bool,
    tool_result_carrier: Option<bool>,
    is_human: Option<bool>,
    pub(crate) has_conflict: bool,
    native_session_id: Option<String>,
    kind: String,
    surface: Option<String>,
    pub(crate) host: Host,
    pub(crate) source: SessionSource,
}
