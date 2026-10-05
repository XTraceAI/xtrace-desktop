//! Structural facts that one indexed user session was created by another
//! session's agent: sub-sessions.
//!
//! Three kinds of proof exist, each with its own insertion path. A Codex
//! thread whose own rollout opens with a `session_meta` naming the thread that
//! spawned it goes through [`Store::record_session_creations`]: native
//! ingestion and the pass over already-indexed sources both use it. A Claude
//! session that a trusted validator proved one Codex agent created with a
//! recorded foreground `claude -p` launch goes through
//! [`Store::record_cli_artifact_creations`], which nothing automatic calls. A
//! Claude session that the native launch scan proved one Codex agent created
//! with a recorded `claude -p --session-id` launch goes through
//! [`Store::record_claude_launch_creations`]. Nothing here accepts or keeps a
//! prompt, command, output, path, task path, role, title or a digest of any
//! of them.
//!
//! A relation is a navigation projection. It never moves or copies work: the
//! child keeps its own row, identity, detail route and every measurement, and
//! no metric, span, search or page-membership query reads this table. The
//! Human input view does read it, for every kind alike: the user messages of
//! a session with an accepted creation relation are taken as sent by the
//! agent that created it, not typed by a person.
//!
//! The store refuses what it cannot tie to one child: a child that is not
//! exactly one indexed user session with the named native identity, a parent
//! that is the child, a parent chain that leads back to the child, and a
//! native child identity already related under another canonical session.
//! Replaying a stored proof changes nothing. A proof that names a different
//! parent for a child already related never replaces the first: that child's
//! relation becomes durably conflicted and is withheld from every read.

use crate::{Error, Host, Result, Store, model::text_enum};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Proofs accepted by one call. A caller with more splits them into batches.
pub const MAX_CREATION_PROOFS: usize = 1_000;
/// Longest accepted structural identifier, in bytes.
const MAX_IDENTIFIER: usize = 256;
/// The longest parent chain a cycle check follows. A longer chain is not
/// known to be acyclic, so a proof that would extend it abstains.
pub const MAX_CREATION_DEPTH: usize = 64;

// The closed set of evidence a relation may rest on. `codex_thread_spawn`: the
// child Codex thread's own rollout opens with a `session_meta` whose
// `payload.id` is the child and whose `source.subagent.thread_spawn` names a
// distinct full parent thread identity. `cli_artifact_create`: a Codex agent's
// recorded foreground Claude CLI launch created the child; see
// [`CliArtifactCreationProof`]. `codex_claude_launch`: a Codex agent's recorded
// `claude -p --session-id` launch named and created the child; see
// [`ClaudeLaunchCreationProof`].
text_enum!(CreationEvidence {
    CodexThreadSpawn => "codex_thread_spawn",
    CliArtifactCreate => "cli_artifact_create",
    CodexClaudeLaunch => "codex_claude_launch",
});

// Which structural record carried the evidence. `rollout_opening_session_meta`:
// the complete first line of the child's own root rollout.
// `claude_cli_redirected_json_result`: the complete JSON result the launched
// process wrote to the file its command redirected output to.
// `codex_exec_session_id_launch`: the parent's own `exec` operation whose
// literal command named the child with `--session-id`, and that operation's
// own recorded process result (from version 3; its exit 0 in version 1).
text_enum!(CreationWitness {
    RolloutOpeningSessionMeta => "rollout_opening_session_meta",
    ClaudeCliRedirectedJsonResult => "claude_cli_redirected_json_result",
    CodexExecSessionIdLaunch => "codex_exec_session_id_launch",
});

/// Version of the native Codex spawn matcher.
pub const CODEX_THREAD_SPAWN_VERSION: u32 = 1;

/// Version of the foreground Claude CLI artifact validator. It is the only one
/// accepted: another launch or result shape needs its own reviewed version.
pub const CLI_ARTIFACT_CREATE_VERSION: u32 = 1;

/// Version of the native Codex launch scan whose proofs
/// [`Store::record_claude_launch_creations`] accepts. It is the only one
/// accepted: another launch form needs its own reviewed version. Version 3
/// rests a creation on the launch operation's own first process result;
/// relations version 1 accepted (on the process's exit 0) stay as stored, and
/// a version 3 proof of the same creation anchor replays them.
pub const CLAUDE_LAUNCH_CREATE_VERSION: u32 = 3;

/// The identities that prove one session created another. Every field is
/// structural; none is transcript content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionCreationProof {
    /// The created session's canonical identity.
    pub child_session_id: String,
    pub child_host: Host,
    /// The created session's native identity, as the evidence named it. It
    /// must equal the indexed session's own.
    pub child_native_session_id: String,
    pub parent_host: Host,
    /// The creating session's full native identity, as the evidence named it.
    /// It need not be indexed.
    pub parent_native_session_id: String,
    pub evidence_kind: CreationEvidence,
    /// Version of the matcher that established the relation.
    pub evidence_version: u32,
    pub witness: CreationWitness,
}

/// The structural chain by which a Codex agent's foreground Claude CLI launch
/// created a Claude session, as a trusted validator established it. Every
/// field is an identifier; none is transcript content.
///
/// The store checks every identity it holds: the child is exactly one indexed
/// Claude user session with this native identity, `first_record_uuid` is that
/// session's own first eligible input (a user record with text that is
/// neither meta, sidechain nor a tool result, with no other such user record,
/// with text or without, at or before it), and the parent is exactly one
/// indexed Codex user session with this canonical and native identity. It
/// cannot check what the index does not hold, so the validator must have
/// proven, before submitting, that:
///
/// - the launch call's operation is one literal create-mode `claude -p`
///   command, with no resume, continue, supplied session or substitution,
///   whose output is redirected to one result file;
/// - that operation's own process handle is `process_session_id`, and
///   `completion_call_id` observed that process exit with status 0;
/// - the complete result file it wrote is a successful Claude result whose
///   `session_id` is the child and whose own identifier is
///   `provider_result_uuid`; the file was written between launch and
///   completion, and `output_read_call_id` read it afterwards with no other
///   writer in between;
/// - the child's first input is exactly, and uniquely, what the launch
///   submitted, compared in memory and never kept.
///
/// This is lineage only. It confirms nothing about who typed an input: no
/// human-input classification, confirmation or measurement reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliArtifactCreationProof {
    /// The created Claude session's canonical identity.
    pub child_session_id: String,
    /// Its native identity, as the result file named it.
    pub child_native_session_id: String,
    /// The launching Codex session's canonical identity. It must be indexed.
    pub parent_session_id: String,
    pub parent_native_session_id: String,
    /// The child's first eligible input record.
    pub first_record_uuid: String,
    /// The parent's tool call that launched the process, and the zero-based
    /// position of the launch among that call's operations.
    pub launch_call_id: String,
    pub launch_operation_index: u32,
    /// The launched process's own handle.
    pub process_session_id: String,
    /// The parent's call that observed the process exit successfully.
    pub completion_call_id: String,
    /// The parent's later call that read the result file.
    pub output_read_call_id: String,
    /// The result's own identifier. A result without one is not this
    /// version's witness.
    pub provider_result_uuid: String,
    pub evidence_version: u32,
}

/// The structural chain by which a Codex agent's own recorded `exec`
/// operation created a Claude session it named, as the native launch scan
/// established it. Every field is an identifier or a position in the
/// parent's history; none is transcript content.
///
/// The store checks every identity it holds, exactly as for
/// [`CliArtifactCreationProof`]: the child is exactly one indexed Claude user
/// session with this native identity, `first_record_uuid` is its own first
/// eligible input, and the parent is exactly one indexed Codex user session
/// with this canonical and native identity. The scan must have proven,
/// before submitting, that:
///
/// - the launch operation is one literal create-mode `claude -p` command with
///   JSON output, one literal inline prompt and `--session-id` naming the
///   child, with no resume, continue, fork, substitution or other option;
/// - the launch lies in the parent's own rows of the physical history file
///   `segment_rollout_id`, at `launch_ordinal` when that history is
///   paginated, and its acknowledgment lies in the same file;
/// - the host acknowledged that operation: the output of
///   `acknowledgment_call_id` (the launch call, or a `wait` on its running
///   cell), at `acknowledgment_ordinal`, holds at the launch's position
///   (`acknowledgment_operation_index`) the operation's own process result —
///   any exit code, or the running process's `process_session_id`. Nothing
///   after it, not the job's later success or failure, is part of the proof;
/// - the child is a fresh session whose first input is exactly what the
///   launch submitted, dated at or after the launch, compared in memory and
///   never kept.
///
/// The creation anchor is the child, the parent, the child's first input, the
/// launch operation, its history file and its row: a proof of the same anchor
/// replays an accepted relation, whatever its acknowledgment or version.
///
/// Like every accepted relation, the Human input view reads it: the child's
/// user messages are taken as sent by the agent that created it. No
/// per-message evidence is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeLaunchCreationProof {
    pub child_session_id: String,
    pub child_native_session_id: String,
    pub parent_session_id: String,
    pub parent_native_session_id: String,
    pub first_record_uuid: String,
    pub launch_call_id: String,
    pub launch_operation_index: u32,
    /// The launched process's handle, when its result said it was running.
    pub process_session_id: Option<String>,
    /// The call whose output held the launch operation's own result, and the
    /// operation's position: stored as `completion_call_id` and
    /// `completion_operation_index` (in version 1, the call that saw the
    /// process exit 0).
    pub acknowledgment_call_id: String,
    pub acknowledgment_operation_index: u32,
    /// The physical history file the launch lies in: the thread's own
    /// identity for its original rollout, a continuation's rollout identity
    /// otherwise.
    pub segment_rollout_id: String,
    /// The launch and acknowledgment rows' ordinals in a paginated history;
    /// both absent in a flat one. The second is stored as
    /// `completion_ordinal`.
    pub launch_ordinal: Option<i64>,
    pub acknowledgment_ordinal: Option<i64>,
    pub evidence_version: u32,
}

/// Why one proof changed nothing. None is an error: the store does not know
/// enough to relate that child, and leaves it a plain session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreationAbstention {
    /// No session has this canonical identity.
    MissingChild,
    /// The session is a judge session, not a user session.
    NotUserSession,
    /// The session's host or native identity is unknown or differs.
    ChildIdentityMismatch,
    /// More than one indexed user session holds the child's native identity.
    AmbiguousChild,
    /// The native child identity is already related under another canonical
    /// session.
    ReusedChild,
    /// The parent is the child.
    SelfLink,
    /// The parent's own stored chain leads back to the child, or is longer
    /// than a check follows.
    Cycle,
    /// The same batch names this child again with a different parent or
    /// identity. None of them is applied.
    AmbiguousInBatch,
    /// The named record is not the child's own first eligible input.
    FirstInputMismatch,
    /// The parent is not an indexed user session of the named host with this
    /// canonical and native identity.
    ParentMismatch,
    /// More than one indexed user session holds the parent's native identity.
    AmbiguousParent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreationDisposition {
    /// Newly stored.
    Recorded,
    /// This child was already related to this parent; nothing changed.
    AlreadyRecorded,
    /// This child's relation disagrees with an earlier one, now or before.
    /// It is withheld from every read and never replaced.
    Conflicted,
    Abstained(CreationAbstention),
}

/// One disposition per supplied proof, in input order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationReport {
    pub dispositions: Vec<CreationDisposition>,
    /// How many proofs changed what is stored, and so what a read shows: a
    /// relation newly recorded, or an accepted one first withheld as
    /// conflicted. A replay, a relation already conflicted and an abstention
    /// change nothing.
    pub changed: usize,
}

/// How far the pass over already-indexed sources has read for one evidence
/// kind and version.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CreationBootstrap {
    /// The last locator the pass finished, in key order; `None` before the
    /// first.
    pub after_locator: Option<String>,
    pub complete: bool,
}

impl Store {
    /// Apply creation proofs in one immediate transaction.
    ///
    /// Malformed identifiers, evidence that does not fit its kind, or too many
    /// proofs reject the whole call before anything is written. Otherwise
    /// every proof receives a disposition and what was stored commits
    /// together.
    pub fn record_session_creations(
        &mut self,
        proofs: &[SessionCreationProof],
        recorded_at: i64,
    ) -> Result<CreationReport> {
        if proofs.len() > MAX_CREATION_PROOFS {
            return Err(Error::InvalidInput(
                "too many session creation proofs in one call",
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
        let mut changed = 0;
        for (index, proof) in proofs.iter().enumerate() {
            dispositions.push(if ambiguous.contains(&index) {
                CreationDisposition::Abstained(CreationAbstention::AmbiguousInBatch)
            } else {
                let (disposition, stored) = apply(&transaction, proof, recorded_at)?;
                changed += usize::from(stored);
                disposition
            });
        }
        transaction.commit()?;
        Ok(CreationReport {
            dispositions,
            changed,
        })
    }

    /// Apply foreground Claude CLI creation proofs in one immediate
    /// transaction. Only a trusted validator that proved every precondition
    /// on [`CliArtifactCreationProof`] may call this; no ingestion, reader or
    /// app command does.
    ///
    /// A malformed identifier, an unknown validator version or too many
    /// proofs reject the whole call before anything is written. Otherwise the
    /// proofs apply in order and commit together. An exact replay changes
    /// nothing. Any different proof for a child already related, including
    /// the same parent through another launch, first input or result,
    /// withholds that child as conflicted. Every valid proof claims its launch
    /// operation first; a launch claimed for a second child is disputed for
    /// good, and every relation resting on it or on its first child, stored
    /// then or later, is withheld.
    pub fn record_cli_artifact_creations(
        &mut self,
        proofs: &[CliArtifactCreationProof],
        recorded_at: i64,
    ) -> Result<CreationReport> {
        if proofs.len() > MAX_CREATION_PROOFS {
            return Err(Error::InvalidInput(
                "too many session creation proofs in one call",
            ));
        }
        for proof in proofs {
            validate_artifact(proof)?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut dispositions = Vec::with_capacity(proofs.len());
        let mut changed = 0;
        for proof in proofs {
            let (disposition, stored) = apply_artifact(&transaction, proof, recorded_at)?;
            changed += usize::from(stored);
            dispositions.push(disposition);
        }
        transaction.commit()?;
        Ok(CreationReport {
            dispositions,
            changed,
        })
    }

    /// The stored foreground CLI relation of one child, whatever its state:
    /// its proof and whether it is withheld as conflicted.
    pub fn cli_artifact_creation(
        &self,
        child_session_id: &str,
    ) -> Result<Option<(CliArtifactCreationProof, bool)>> {
        Ok(stored_artifact(&self.connection, child_session_id)?
            .and_then(|(proof, state)| Some((proof?, state == "conflicted"))))
    }

    /// Apply native Codex launch proofs in one immediate transaction. Only
    /// the native launch scan, having proved every precondition on
    /// [`ClaudeLaunchCreationProof`], calls this.
    ///
    /// A malformed identifier, an unknown version or too many proofs reject
    /// the whole call before anything is written. Otherwise the proofs apply
    /// in order and commit together, under the same guards as
    /// [`Self::record_cli_artifact_creations`]: an exact replay changes
    /// nothing, and so does a proof of the same creation anchor as the
    /// child's accepted relation — an earlier launch version's with the same
    /// child, parent, first input, launch operation, history file and launch
    /// row, or a foreground CLI relation with the same child, parent, first
    /// input and launch operation — leaving that relation exactly as stored;
    /// any other proof for a child already related, of this or another kind,
    /// withholds that child as conflicted; and a launch operation claimed for
    /// a second child, by either CLI kind, is disputed for good and withholds
    /// every relation resting on it or on its first child.
    pub fn record_claude_launch_creations(
        &mut self,
        proofs: &[ClaudeLaunchCreationProof],
        recorded_at: i64,
    ) -> Result<CreationReport> {
        if proofs.len() > MAX_CREATION_PROOFS {
            return Err(Error::InvalidInput(
                "too many session creation proofs in one call",
            ));
        }
        for proof in proofs {
            validate_launch(proof)?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut dispositions = Vec::with_capacity(proofs.len());
        let mut changed = 0;
        for proof in proofs {
            let (disposition, stored) = apply_launch(&transaction, proof, recorded_at)?;
            changed += usize::from(stored);
            dispositions.push(disposition);
        }
        transaction.commit()?;
        Ok(CreationReport {
            dispositions,
            changed,
        })
    }

    /// The stored native launch relation of one child, whatever its state:
    /// its proof and whether it is withheld as conflicted.
    pub fn claude_launch_creation(
        &self,
        child_session_id: &str,
    ) -> Result<Option<(ClaudeLaunchCreationProof, bool)>> {
        Ok(stored_launch(&self.connection, child_session_id)?
            .and_then(|(proof, state)| Some((proof?, state == "conflicted"))))
    }

    /// The stored relation of one child, whatever its state: its proof and
    /// whether it is withheld as conflicted.
    pub fn session_creation(
        &self,
        child_session_id: &str,
    ) -> Result<Option<(SessionCreationProof, bool)>> {
        Ok(self
            .connection
            .query_row(
                &format!(
                    "SELECT {PROOF_FIELDS},state FROM session_creation_relations
                     WHERE child_session_id=?1"
                ),
                [child_session_id],
                |row| {
                    Ok((
                        proof_from_row(row)?,
                        row.get::<_, String>(8)? == "conflicted",
                    ))
                },
            )
            .optional()?)
    }

    /// How far the pass over already-indexed sources has read.
    pub fn session_creation_bootstrap(
        &self,
        kind: CreationEvidence,
        version: u32,
    ) -> Result<CreationBootstrap> {
        Ok(self
            .connection
            .query_row(
                "SELECT after_locator,complete FROM session_creation_bootstrap
                 WHERE evidence_kind=?1 AND evidence_version=?2",
                params![kind, version],
                |row| {
                    Ok(CreationBootstrap {
                        after_locator: row.get(0)?,
                        complete: row.get(1)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Record that the pass finished every locator up to `after_locator`, and
    /// whether it reached the end. Progress never moves back, and a completed
    /// pass stays complete.
    pub fn advance_session_creation_bootstrap(
        &mut self,
        kind: CreationEvidence,
        version: u32,
        progress: &CreationBootstrap,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO session_creation_bootstrap(evidence_kind,evidence_version,
                 after_locator,complete) VALUES (?1,?2,?3,?4)
             ON CONFLICT(evidence_kind,evidence_version) DO UPDATE SET
                 after_locator=CASE
                     WHEN session_creation_bootstrap.after_locator IS NULL
                       OR excluded.after_locator > session_creation_bootstrap.after_locator
                     THEN excluded.after_locator
                     ELSE session_creation_bootstrap.after_locator END,
                 complete=max(session_creation_bootstrap.complete,excluded.complete)",
            params![kind, version, progress.after_locator, progress.complete],
        )?;
        Ok(())
    }

    /// Native source locator keys of `source` that start with `prefix`, after
    /// `after` in key order, at most `limit` of them. Keys only: a locator
    /// names a local path the index read, never content. The primary key
    /// orders and bounds the read.
    pub fn source_locator_keys_after(
        &self,
        source: crate::SessionSource,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>> {
        let Some(last) = prefix.chars().last() else {
            return Err(Error::InvalidInput("locator prefix must not be empty"));
        };
        // Every key starting with `prefix` sorts at or after it and before the
        // prefix with its last character advanced by one.
        let mut end = prefix[..prefix.len() - last.len_utf8()].to_owned();
        end.push(
            char::from_u32(u32::from(last) + 1)
                .ok_or(Error::InvalidInput("locator prefix cannot be bounded"))?,
        );
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        Ok(self
            .connection
            .prepare(
                "SELECT cursor_key FROM source_cursors WHERE source=?1
                   AND cursor_key>=?2 AND cursor_key<?3 AND (?4 IS NULL OR cursor_key>?4)
                 ORDER BY cursor_key LIMIT ?5",
            )?
            .query_map(params![source, prefix, end, after, limit], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Native source locator keys of `source` whose last [`LOCATOR_TAIL`]
    /// characters are exactly `tail`, in key order, at most `limit` of them.
    /// An index on that tail serves the lookup, so it seeks the matches
    /// rather than reading every locator.
    pub fn source_locator_keys_ending(
        &self,
        source: crate::SessionSource,
        tail: &str,
        limit: usize,
    ) -> Result<Vec<String>> {
        if tail.chars().count() != LOCATOR_TAIL {
            return Err(Error::InvalidInput(
                "a locator tail must be LOCATOR_TAIL characters",
            ));
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        Ok(self
            .connection
            .prepare(LOCATOR_TAIL_QUERY)?
            .query_map(params![source, tail, limit], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The canonical identities of the indexed user sessions of `host` whose
    /// native identity is exactly `native`: none, one, or an ambiguity.
    pub fn user_sessions_with_native(&self, host: Host, native: &str) -> Result<Vec<String>> {
        user_sessions_with_native(&self.connection, host, native)
    }
}

/// Characters at the end of a locator key that
/// [`Store::source_locator_keys_ending`] matches: a full Codex thread identity
/// and `.jsonl`, which is how a root rollout's name ends.
pub const LOCATOR_TAIL: usize = 42;

/// The expression must be the indexed one, `substr(cursor_key, -42)`, for the
/// index on it to serve the lookup.
pub const LOCATOR_TAIL_QUERY: &str = "SELECT cursor_key FROM source_cursors
     WHERE source=?1 AND substr(cursor_key, -42)=?2 ORDER BY cursor_key LIMIT ?3";

const PROOF_FIELDS: &str = "child_session_id,child_host,child_native_session_id,parent_host,
    parent_native_session_id,evidence_kind,evidence_version,witness";

fn proof_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionCreationProof> {
    Ok(SessionCreationProof {
        child_session_id: row.get(0)?,
        child_host: row.get(1)?,
        child_native_session_id: row.get(2)?,
        parent_host: row.get(3)?,
        parent_native_session_id: row.get(4)?,
        evidence_kind: row.get(5)?,
        evidence_version: row.get(6)?,
        witness: row.get(7)?,
    })
}

pub(crate) fn user_sessions_with_native(
    connection: &Connection,
    host: Host,
    native: &str,
) -> Result<Vec<String>> {
    Ok(connection
        .prepare(
            "SELECT session_id FROM sessions WHERE host=?1 AND native_session_id=?2
               AND kind='user' ORDER BY session_id LIMIT 2",
        )?
        .query_map(params![host, native], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn identifier(value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > MAX_IDENTIFIER
        || value.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput(
            "session creation proof requires bounded structural identifiers",
        ));
    }
    Ok(())
}

fn validate(proof: &SessionCreationProof) -> Result<()> {
    for value in [
        &proof.child_session_id,
        &proof.child_native_session_id,
        &proof.parent_native_session_id,
    ] {
        identifier(value)?;
    }
    if proof.evidence_version == 0 {
        return Err(Error::InvalidInput(
            "session creation proof requires an evidence version",
        ));
    }
    let fits = match proof.evidence_kind {
        CreationEvidence::CodexThreadSpawn => {
            proof.child_host == Host::Codex
                && proof.parent_host == Host::Codex
                && proof.witness == CreationWitness::RolloutOpeningSessionMeta
        }
        // Only their own paths, with their whole witness, record these kinds.
        CreationEvidence::CliArtifactCreate | CreationEvidence::CodexClaudeLaunch => false,
    };
    if !fits {
        return Err(Error::InvalidInput(
            "session creation proof does not fit its evidence kind",
        ));
    }
    Ok(())
}

/// Indexes of proofs the batch contradicts: one child named with any
/// different parent or identity, or one native child named under two
/// canonical sessions. Exact repeats are not contradictions; the first
/// applies and the rest find it stored.
fn ambiguous(proofs: &[SessionCreationProof]) -> BTreeSet<usize> {
    let mut by_child = BTreeMap::<&str, Vec<usize>>::new();
    let mut by_native = BTreeMap::<(&str, &str), Vec<usize>>::new();
    for (index, proof) in proofs.iter().enumerate() {
        by_child
            .entry(&proof.child_session_id)
            .or_default()
            .push(index);
        by_native
            .entry((proof.child_host.as_str(), &proof.child_native_session_id))
            .or_default()
            .push(index);
    }
    let same = |a: &SessionCreationProof, b: &SessionCreationProof| {
        a.child_session_id == b.child_session_id
            && a.child_host == b.child_host
            && a.child_native_session_id == b.child_native_session_id
            && a.parent_host == b.parent_host
            && a.parent_native_session_id == b.parent_native_session_id
    };
    by_child
        .into_values()
        .chain(by_native.into_values())
        .filter(|group| group.iter().any(|&i| !same(&proofs[i], &proofs[group[0]])))
        .flatten()
        .collect()
}

/// A proof's disposition, and whether applying it changed what is stored.
fn apply(
    connection: &Connection,
    proof: &SessionCreationProof,
    recorded_at: i64,
) -> Result<(CreationDisposition, bool)> {
    let abstain = |reason| Ok((CreationDisposition::Abstained(reason), false));
    if proof.parent_host == proof.child_host
        && proof
            .parent_native_session_id
            .eq_ignore_ascii_case(&proof.child_native_session_id)
    {
        return abstain(CreationAbstention::SelfLink);
    }
    if let Some(reason) = child_abstention(
        connection,
        &proof.child_session_id,
        proof.child_host,
        &proof.child_native_session_id,
    )? {
        return abstain(reason);
    }
    let stored = connection
        .query_row(
            &format!(
                "SELECT {PROOF_FIELDS},state FROM session_creation_relations
                 WHERE child_session_id=?1"
            ),
            [&proof.child_session_id],
            |row| Ok((proof_from_row(row)?, row.get::<_, String>(8)?)),
        )
        .optional()?;
    if let Some((stored, state)) = stored {
        let agrees = stored.child_host == proof.child_host
            && stored.child_native_session_id == proof.child_native_session_id
            && stored.parent_host == proof.parent_host
            && stored.parent_native_session_id == proof.parent_native_session_id;
        if agrees && state == "accepted" {
            return Ok((CreationDisposition::AlreadyRecorded, false));
        }
        let first = state == "accepted";
        if first {
            connection.execute(
                "UPDATE session_creation_relations SET state='conflicted'
                 WHERE child_session_id=?1",
                [&proof.child_session_id],
            )?;
        }
        return Ok((CreationDisposition::Conflicted, first));
    }
    let reused: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_creation_relations
           WHERE child_host=?1 AND child_native_session_id=?2)",
        params![proof.child_host, proof.child_native_session_id],
        |row| row.get(0),
    )?;
    if reused {
        return abstain(CreationAbstention::ReusedChild);
    }
    if leads_back(
        connection,
        (proof.child_host, &proof.child_native_session_id),
        (proof.parent_host, &proof.parent_native_session_id),
    )? {
        return abstain(CreationAbstention::Cycle);
    }
    connection.execute(
        "INSERT INTO session_creation_relations(child_session_id,child_host,
             child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
             evidence_version,witness,state,recorded_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'accepted',?9)",
        params![
            proof.child_session_id,
            proof.child_host,
            proof.child_native_session_id,
            proof.parent_host,
            proof.parent_native_session_id,
            proof.evidence_kind,
            proof.evidence_version,
            proof.witness,
            recorded_at,
        ],
    )?;
    Ok((CreationDisposition::Recorded, true))
}

/// Why a named child cannot be related, if it cannot: it must be exactly one
/// indexed user session of `host` with exactly this native identity.
fn child_abstention(
    connection: &Connection,
    child_session_id: &str,
    child_host: Host,
    child_native: &str,
) -> Result<Option<CreationAbstention>> {
    let child = connection
        .query_row(
            "SELECT host,native_session_id,kind FROM sessions WHERE session_id=?1",
            [child_session_id],
            |row| {
                Ok((
                    row.get::<_, Host>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((host, native, kind)) = child else {
        return Ok(Some(CreationAbstention::MissingChild));
    };
    if kind != "user" {
        return Ok(Some(CreationAbstention::NotUserSession));
    }
    if host != child_host || native.as_deref() != Some(child_native) {
        return Ok(Some(CreationAbstention::ChildIdentityMismatch));
    }
    if user_sessions_with_native(connection, host, child_native)?.len() != 1 {
        return Ok(Some(CreationAbstention::AmbiguousChild));
    }
    Ok(None)
}

/// Whether the parent's stored chain, followed by native identity whatever
/// each relation's state, reaches the child, or runs longer than a check
/// follows. Native identities are followed so that a parent not indexed yet
/// still closes a cycle.
fn leads_back(
    connection: &Connection,
    (child_host, child_native): (Host, &str),
    (parent_host, parent_native): (Host, &str),
) -> Result<bool> {
    let mut host = parent_host;
    let mut native = parent_native.to_owned();
    for _ in 0..MAX_CREATION_DEPTH {
        if host == child_host && native.eq_ignore_ascii_case(child_native) {
            return Ok(true);
        }
        let next = connection
            .query_row(
                "SELECT parent_host,parent_native_session_id FROM session_creation_relations
                 WHERE child_host=?1 AND child_native_session_id=?2",
                params![host, native],
                |row| Ok((row.get::<_, Host>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        match next {
            Some((parent_host, parent_native)) => {
                host = parent_host;
                native = parent_native;
            }
            None => return Ok(false),
        }
    }
    Ok(true)
}

/// A structural token of the CLI witness: 1 to 256 printable ASCII characters
/// with no space, the same rule the table enforces.
fn token(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(Error::InvalidInput(
            "a foreground CLI creation proof requires bounded structural tokens",
        ));
    }
    Ok(())
}

fn validate_artifact(proof: &CliArtifactCreationProof) -> Result<()> {
    for value in [
        &proof.child_session_id,
        &proof.child_native_session_id,
        &proof.parent_session_id,
        &proof.parent_native_session_id,
        &proof.first_record_uuid,
        &proof.launch_call_id,
        &proof.process_session_id,
        &proof.completion_call_id,
        &proof.output_read_call_id,
        &proof.provider_result_uuid,
    ] {
        token(value)?;
    }
    if proof.evidence_version != CLI_ARTIFACT_CREATE_VERSION {
        return Err(Error::InvalidInput(
            "a foreground CLI creation proof requires a known validator version",
        ));
    }
    Ok(())
}

const ARTIFACT_FIELDS: &str = "evidence_kind,witness,child_session_id,child_native_session_id,
    parent_session_id,parent_native_session_id,first_record_uuid,launch_call_id,
    launch_operation_index,process_session_id,completion_call_id,output_read_call_id,
    provider_result_uuid,evidence_version,state";

/// The stored relation of one child as a CLI proof, and its state. The proof
/// is `None` when the stored row rests on another kind of evidence.
fn stored_artifact(
    connection: &Connection,
    child_session_id: &str,
) -> Result<Option<(Option<CliArtifactCreationProof>, String)>> {
    Ok(connection
        .query_row(
            &format!(
                "SELECT {ARTIFACT_FIELDS} FROM session_creation_relations
                 WHERE child_session_id=?1"
            ),
            [child_session_id],
            |row| {
                let kind: CreationEvidence = row.get(0)?;
                let witness: CreationWitness = row.get(1)?;
                let state: String = row.get(14)?;
                if kind != CreationEvidence::CliArtifactCreate
                    || witness != CreationWitness::ClaudeCliRedirectedJsonResult
                {
                    return Ok((None, state));
                }
                Ok((
                    Some(CliArtifactCreationProof {
                        child_session_id: row.get(2)?,
                        child_native_session_id: row.get(3)?,
                        parent_session_id: row.get(4)?,
                        parent_native_session_id: row.get(5)?,
                        first_record_uuid: row.get(6)?,
                        launch_call_id: row.get(7)?,
                        launch_operation_index: row.get(8)?,
                        process_session_id: row.get(9)?,
                        completion_call_id: row.get(10)?,
                        output_read_call_id: row.get(11)?,
                        provider_result_uuid: row.get(12)?,
                        evidence_version: row.get(13)?,
                    }),
                    state,
                ))
            },
        )
        .optional()?)
}

/// Whether `record_uuid` is the child's own first eligible input: a timed
/// user record it owns that is neither meta, sidechain nor a tool result and
/// carries text, with no other such user record, with text or without,
/// untimed or at or before it. An earlier input with no text, an image alone,
/// was still saved first, so a later text input is not the first.
fn first_input(connection: &Connection, child_session_id: &str, record_uuid: &str) -> Result<bool> {
    const ELIGIBLE: &str = "type='user' AND role='user' AND is_meta=0 AND is_sidechain=0
         AND is_tool_result_carrier=0";
    let at: Option<Option<i64>> = connection
        .query_row(
            &format!(
                "SELECT ts_ms FROM records WHERE uuid=?1 AND session_id=?2 AND {ELIGIBLE}
                   AND text_len>0"
            ),
            params![record_uuid, child_session_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(Some(at)) = at else {
        return Ok(false);
    };
    let earlier: bool = connection.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM records WHERE session_id=?1 AND uuid<>?2 AND {ELIGIBLE}
               AND (ts_ms IS NULL OR ts_ms<=?3))"
        ),
        params![child_session_id, record_uuid, at],
        |row| row.get(0),
    )?;
    Ok(!earlier)
}

/// The identities every Codex-to-Claude CLI proof names, whatever its kind.
struct CliClaim<'a> {
    child_session_id: &'a str,
    child_native_session_id: &'a str,
    parent_session_id: &'a str,
    parent_native_session_id: &'a str,
    first_record_uuid: &'a str,
    launch_call_id: &'a str,
    launch_operation_index: u32,
}

impl<'a> From<&'a CliArtifactCreationProof> for CliClaim<'a> {
    fn from(proof: &'a CliArtifactCreationProof) -> Self {
        Self {
            child_session_id: &proof.child_session_id,
            child_native_session_id: &proof.child_native_session_id,
            parent_session_id: &proof.parent_session_id,
            parent_native_session_id: &proof.parent_native_session_id,
            first_record_uuid: &proof.first_record_uuid,
            launch_call_id: &proof.launch_call_id,
            launch_operation_index: proof.launch_operation_index,
        }
    }
}

impl<'a> From<&'a ClaudeLaunchCreationProof> for CliClaim<'a> {
    fn from(proof: &'a ClaudeLaunchCreationProof) -> Self {
        Self {
            child_session_id: &proof.child_session_id,
            child_native_session_id: &proof.child_native_session_id,
            parent_session_id: &proof.parent_session_id,
            parent_native_session_id: &proof.parent_native_session_id,
            first_record_uuid: &proof.first_record_uuid,
            launch_call_id: &proof.launch_call_id,
            launch_operation_index: proof.launch_operation_index,
        }
    }
}

/// Whether two CLI claims name the same child, parent, first input and
/// launch operation.
fn same_claim(a: &CliClaim<'_>, b: &CliClaim<'_>) -> bool {
    a.child_session_id == b.child_session_id
        && a.child_native_session_id == b.child_native_session_id
        && a.parent_session_id == b.parent_session_id
        && a.parent_native_session_id == b.parent_native_session_id
        && a.first_record_uuid == b.first_record_uuid
        && a.launch_call_id == b.launch_call_id
        && a.launch_operation_index == b.launch_operation_index
}

/// Why a CLI claim cannot be related, checked before it claims anything: the
/// same checks for every CLI kind.
fn cli_abstention(
    connection: &Connection,
    proof: &CliClaim<'_>,
) -> Result<Option<CreationAbstention>> {
    if proof.parent_session_id == proof.child_session_id {
        return Ok(Some(CreationAbstention::SelfLink));
    }
    if let Some(reason) = child_abstention(
        connection,
        proof.child_session_id,
        Host::Claude,
        proof.child_native_session_id,
    )? {
        return Ok(Some(reason));
    }
    // A record the child does not own, or not its first input, is not this
    // child's creation anchor: it abstains without touching a stored claim.
    if !first_input(connection, proof.child_session_id, proof.first_record_uuid)? {
        return Ok(Some(CreationAbstention::FirstInputMismatch));
    }
    let parent = connection
        .query_row(
            "SELECT host,native_session_id,kind FROM sessions WHERE session_id=?1",
            [proof.parent_session_id],
            |row| {
                Ok((
                    row.get::<_, Host>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    if parent
        != Some((
            Host::Codex,
            Some(proof.parent_native_session_id.to_owned()),
            "user".to_owned(),
        ))
    {
        return Ok(Some(CreationAbstention::ParentMismatch));
    }
    if user_sessions_with_native(connection, Host::Codex, proof.parent_native_session_id)?.len()
        != 1
    {
        return Ok(Some(CreationAbstention::AmbiguousParent));
    }
    // A native child already related under another canonical session, or a
    // parent chain leading back to the child, abstains here, so a proof is
    // fully validated before it claims anything.
    let reused: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_creation_relations
           WHERE child_host='claude' AND child_native_session_id=?1 AND child_session_id<>?2)",
        params![proof.child_native_session_id, proof.child_session_id],
        |row| row.get(0),
    )?;
    if reused {
        return Ok(Some(CreationAbstention::ReusedChild));
    }
    if leads_back(
        connection,
        (Host::Claude, proof.child_native_session_id),
        (Host::Codex, proof.parent_native_session_id),
    )? {
        return Ok(Some(CreationAbstention::Cycle));
    }
    Ok(None)
}

/// A CLI proof's disposition, and whether applying it changed what is stored.
fn apply_artifact(
    connection: &Connection,
    proof: &CliArtifactCreationProof,
    recorded_at: i64,
) -> Result<(CreationDisposition, bool)> {
    if let Some(reason) = cli_abstention(connection, &proof.into())? {
        return Ok((CreationDisposition::Abstained(reason), false));
    }
    // Every valid proof claims its launch first, whatever becomes of its
    // relation, so a launch this child's stored row never kept is still known
    // to be this child's.
    let (owned, withheld) = claim_launch(connection, &proof.into())?;
    if let Some((stored, state)) = stored_artifact(connection, &proof.child_session_id)? {
        if owned && state == "accepted" && stored.as_ref() == Some(proof) {
            return Ok((CreationDisposition::AlreadyRecorded, false));
        }
        // A second anchor for this child, or a disputed launch, contradicts
        // its stored claim.
        let first = connection.execute(
            "UPDATE session_creation_relations SET state='conflicted'
             WHERE child_session_id=?1 AND state='accepted'",
            [&proof.child_session_id],
        )? > 0;
        return Ok((CreationDisposition::Conflicted, first || withheld > 0));
    }
    connection.execute(
        "INSERT INTO session_creation_relations(child_session_id,child_host,
             child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
             evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
             launch_call_id,launch_operation_index,process_session_id,completion_call_id,
             output_read_call_id,provider_result_uuid)
         VALUES (?1,'claude',?2,'codex',?3,'cli_artifact_create',?4,
             'claude_cli_redirected_json_result',?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            proof.child_session_id,
            proof.child_native_session_id,
            proof.parent_native_session_id,
            proof.evidence_version,
            if owned { "accepted" } else { "conflicted" },
            recorded_at,
            proof.parent_session_id,
            proof.first_record_uuid,
            proof.launch_call_id,
            proof.launch_operation_index,
            proof.process_session_id,
            proof.completion_call_id,
            proof.output_read_call_id,
            proof.provider_result_uuid,
        ],
    )?;
    Ok(if owned {
        (CreationDisposition::Recorded, true)
    } else {
        (CreationDisposition::Conflicted, withheld > 0)
    })
}

/// Claim a valid proof's launch operation for its child. One launch created
/// one child: the first child claimed keeps it, and a claim for any other
/// child marks it disputed for good, withholding every accepted relation of
/// either CLI kind resting on that launch or on its first child, since
/// nothing says which claim is wrong. Returns whether the child holds the
/// launch undisputed, and how many relations a new dispute withheld.
fn claim_launch(connection: &Connection, proof: &CliClaim<'_>) -> Result<(bool, usize)> {
    let launch = params![
        proof.parent_session_id,
        proof.launch_call_id,
        proof.launch_operation_index
    ];
    let owner = connection
        .query_row(
            "SELECT first_child_session_id,disputed FROM cli_artifact_launch_owners
             WHERE parent_session_id=?1 AND launch_call_id=?2 AND launch_operation_index=?3",
            launch,
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
        )
        .optional()?;
    match owner {
        None => {
            connection.execute(
                "INSERT INTO cli_artifact_launch_owners(parent_session_id,launch_call_id,
                     launch_operation_index,first_child_session_id,disputed)
                 VALUES (?1,?2,?3,?4,0)",
                params![
                    proof.parent_session_id,
                    proof.launch_call_id,
                    proof.launch_operation_index,
                    proof.child_session_id
                ],
            )?;
            Ok((true, 0))
        }
        Some((first, false)) if first == proof.child_session_id => Ok((true, 0)),
        Some((_, true)) => Ok((false, 0)),
        Some((first, false)) => {
            connection.execute(
                "UPDATE cli_artifact_launch_owners SET disputed=1
                 WHERE parent_session_id=?1 AND launch_call_id=?2 AND launch_operation_index=?3",
                launch,
            )?;
            let withheld = connection.execute(
                "UPDATE session_creation_relations SET state='conflicted'
                 WHERE evidence_kind IN ('cli_artifact_create','codex_claude_launch')
                   AND state='accepted'
                   AND ((parent_session_id=?1 AND launch_call_id=?2 AND launch_operation_index=?3)
                        OR child_session_id=?4)",
                params![
                    proof.parent_session_id,
                    proof.launch_call_id,
                    proof.launch_operation_index,
                    first
                ],
            )?;
            Ok((false, withheld))
        }
    }
}

pub(crate) fn validate_launch(proof: &ClaudeLaunchCreationProof) -> Result<()> {
    for value in [
        &proof.child_session_id,
        &proof.child_native_session_id,
        &proof.parent_session_id,
        &proof.parent_native_session_id,
        &proof.first_record_uuid,
        &proof.launch_call_id,
        &proof.acknowledgment_call_id,
        &proof.segment_rollout_id,
    ] {
        token(value)?;
    }
    if let Some(handle) = &proof.process_session_id {
        token(handle)?;
    }
    let rollout = &proof.segment_rollout_id;
    let ordinals = match (proof.launch_ordinal, proof.acknowledgment_ordinal) {
        (None, None) => true,
        (Some(launch), Some(acknowledgment)) => launch >= 0 && acknowledgment > launch,
        _ => false,
    };
    if rollout.len() != 36
        || !rollout
            .bytes()
            .all(|byte| byte == b'-' || byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !ordinals
    {
        return Err(Error::InvalidInput(
            "a native launch creation proof requires a rollout identity and ordered ordinals",
        ));
    }
    if proof.evidence_version != CLAUDE_LAUNCH_CREATE_VERSION {
        return Err(Error::InvalidInput(
            "a native launch creation proof requires a known validator version",
        ));
    }
    Ok(())
}

const LAUNCH_FIELDS: &str = "evidence_kind,witness,child_session_id,child_native_session_id,
    parent_session_id,parent_native_session_id,first_record_uuid,launch_call_id,
    launch_operation_index,process_session_id,completion_call_id,completion_operation_index,
    segment_rollout_id,launch_ordinal,completion_ordinal,evidence_version,state";

/// The stored relation of one child as a native launch proof, and its state.
/// The proof is `None` when the stored row rests on another kind of evidence.
fn stored_launch(
    connection: &Connection,
    child_session_id: &str,
) -> Result<Option<(Option<ClaudeLaunchCreationProof>, String)>> {
    Ok(connection
        .query_row(
            &format!(
                "SELECT {LAUNCH_FIELDS} FROM session_creation_relations
                 WHERE child_session_id=?1"
            ),
            [child_session_id],
            |row| {
                let kind: CreationEvidence = row.get(0)?;
                let witness: CreationWitness = row.get(1)?;
                let state: String = row.get(16)?;
                if kind != CreationEvidence::CodexClaudeLaunch
                    || witness != CreationWitness::CodexExecSessionIdLaunch
                {
                    return Ok((None, state));
                }
                Ok((
                    Some(ClaudeLaunchCreationProof {
                        child_session_id: row.get(2)?,
                        child_native_session_id: row.get(3)?,
                        parent_session_id: row.get(4)?,
                        parent_native_session_id: row.get(5)?,
                        first_record_uuid: row.get(6)?,
                        launch_call_id: row.get(7)?,
                        launch_operation_index: row.get(8)?,
                        process_session_id: row.get(9)?,
                        acknowledgment_call_id: row.get(10)?,
                        acknowledgment_operation_index: row.get(11)?,
                        segment_rollout_id: row.get(12)?,
                        launch_ordinal: row.get(13)?,
                        acknowledgment_ordinal: row.get(14)?,
                        evidence_version: row.get(15)?,
                    }),
                    state,
                ))
            },
        )
        .optional()?)
}

/// A native launch proof's disposition, and whether applying it changed what
/// is stored. The same order as a foreground CLI proof: every check, then the
/// launch claim, then the child's stored claim.
pub(crate) fn apply_launch(
    connection: &Connection,
    proof: &ClaudeLaunchCreationProof,
    recorded_at: i64,
) -> Result<(CreationDisposition, bool)> {
    if let Some(reason) = cli_abstention(connection, &proof.into())? {
        return Ok((CreationDisposition::Abstained(reason), false));
    }
    let (owned, withheld) = claim_launch(connection, &proof.into())?;
    if let Some((stored, state)) = stored_launch(connection, &proof.child_session_id)? {
        // The same creation anchor, as an earlier launch relation or a
        // foreground CLI one: the stored relation stands exactly as it is.
        let same = match &stored {
            Some(stored) => {
                same_claim(&stored.into(), &proof.into())
                    && stored.segment_rollout_id == proof.segment_rollout_id
                    && stored.launch_ordinal == proof.launch_ordinal
            }
            None => stored_artifact(connection, &proof.child_session_id)?
                .and_then(|(artifact, _)| artifact)
                .is_some_and(|artifact| same_claim(&(&artifact).into(), &proof.into())),
        };
        if owned && state == "accepted" && same {
            return Ok((CreationDisposition::AlreadyRecorded, false));
        }
        // Another anchor, another kind of evidence or a disputed launch
        // contradicts the child's stored claim.
        let first = connection.execute(
            "UPDATE session_creation_relations SET state='conflicted'
             WHERE child_session_id=?1 AND state='accepted'",
            [&proof.child_session_id],
        )? > 0;
        return Ok((CreationDisposition::Conflicted, first || withheld > 0));
    }
    connection.execute(
        "INSERT INTO session_creation_relations(child_session_id,child_host,
             child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
             evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
             launch_call_id,launch_operation_index,process_session_id,completion_call_id,
             completion_operation_index,segment_rollout_id,launch_ordinal,completion_ordinal)
         VALUES (?1,'claude',?2,'codex',?3,'codex_claude_launch',?4,
             'codex_exec_session_id_launch',?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            proof.child_session_id,
            proof.child_native_session_id,
            proof.parent_native_session_id,
            proof.evidence_version,
            if owned { "accepted" } else { "conflicted" },
            recorded_at,
            proof.parent_session_id,
            proof.first_record_uuid,
            proof.launch_call_id,
            proof.launch_operation_index,
            proof.process_session_id,
            proof.acknowledgment_call_id,
            proof.acknowledgment_operation_index,
            proof.segment_rollout_id,
            proof.launch_ordinal,
            proof.acknowledgment_ordinal,
        ],
    )?;
    Ok(if owned {
        (CreationDisposition::Recorded, true)
    } else {
        (CreationDisposition::Conflicted, withheld > 0)
    })
}
