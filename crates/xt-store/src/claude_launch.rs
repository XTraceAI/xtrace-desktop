//! What the native Codex launch scan keeps between passes: one published
//! validation per Codex thread's whole history (its revision, status and the
//! exact member files with their generations), the launches that validation
//! found with their parent-side verdict and child-side state, and an
//! attempt's staged launches before publication.
//!
//! A validation attempt first marks the thread `pending` and takes a new
//! revision number ([`Store::begin_claude_launch_validation`]); while pending
//! nothing of the thread is linked. It stages its launches in bounded chunks
//! ([`Store::stage_claude_launch_candidates`]) and publishes them, the member
//! set and the verdict in one transaction
//! ([`Store::publish_claude_launch_validation`]) only if the thread still
//! holds the revision the attempt started from and the attempt's own number.
//! A link is written only through [`Store::record_claude_launch_guarded`],
//! which re-checks, in its own transaction, that the published revision, its
//! member generations and the candidate are exactly what the resolution
//! checked.
//!
//! Only identifiers, closed labels, byte positions, ordinals and file
//! generations are kept: never a path, prompt, command, output, time or
//! digest.

use crate::{
    Error, Result, Store,
    creation::{ClaudeLaunchCreationProof, CreationReport, apply_launch, validate_launch},
    model::text_enum,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

/// Rows one read or write handles at most.
pub const MAX_CANDIDATE_ROWS: usize = 1_000;

// A thread's published validation: `pending` while a replacement runs (and
// before the first), `valid`, or `invalid` (nothing of it is linked).
text_enum!(GroupStatus {
    Pending => "pending",
    Valid => "valid",
    Invalid => "invalid",
});

// The child side of a launch: `waiting` (not indexed), `retry` (a source was
// busy or changed), `linked` (a proof went to the store), `rejected` (the
// child's transcript disagrees).
text_enum!(ChildState {
    Waiting => "waiting",
    Retry => "retry",
    Linked => "linked",
    Rejected => "rejected",
});

// The parent side of a launch: `valid` as its validation found it, or
// `rejected` when its recorded lines no longer read as that launch.
text_enum!(SourceVerdict {
    Valid => "valid",
    Rejected => "rejected",
});

/// A file's generation as a stat reports it: which file, how long, and when
/// it last changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SegmentGeneration {
    pub device: i64,
    pub inode: i64,
    pub length: i64,
    pub mtime_ns: i64,
    pub ctime_ns: i64,
}

/// Which launch operation a candidate is.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LaunchKey {
    pub parent_native_session_id: String,
    pub rollout_id: String,
    pub launch_call_id: String,
    pub launch_operation_index: u32,
}

/// One launch's structural fields, as a validation found it: the launch,
/// and its acknowledgment — the output row holding the launch operation's
/// own first process result, the call it answered (the launch call, or a
/// `wait` on its cell) and the launch's operation position. They are kept in
/// the `completion_*` columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchCandidate {
    pub key: LaunchKey,
    pub child_native_session_id: String,
    pub acknowledgment_call_id: String,
    pub acknowledgment_operation_index: u32,
    /// The process handle, when the result said the process was running.
    pub process_session_id: Option<String>,
    pub launch_offset: i64,
    pub acknowledgment_offset: i64,
    pub launch_ordinal: Option<i64>,
    pub acknowledgment_ordinal: Option<i64>,
}

/// A published launch and its two verdicts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateRow {
    pub candidate: LaunchCandidate,
    pub source_revision: i64,
    pub source_verdict: SourceVerdict,
    pub child_state: ChildState,
}

/// A thread's published validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupRecord {
    pub revision: Option<i64>,
    pub allocated: i64,
    pub status: GroupStatus,
    pub validator_version: u32,
}

/// One member file of a published validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRecord {
    pub rollout_id: String,
    pub generation: SegmentGeneration,
    pub scanned_length: i64,
    pub valid: bool,
}

/// What an attempt started from and its own revision number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub expected: Option<i64>,
    pub allocated: i64,
}

/// Counts for acceptance reports. Numbers only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchSummary {
    pub relations_accepted: u64,
    pub relations_conflicted: u64,
    pub groups_valid: u64,
    pub groups_invalid: u64,
    pub groups_pending: u64,
    pub members: u64,
    pub members_invalid: u64,
    /// Bytes of history the published members' complete lines cover.
    pub bytes_covered: u64,
    pub candidates_waiting: u64,
    pub candidates_retry: u64,
    pub candidates_linked: u64,
    pub candidates_rejected: u64,
    pub candidates_source_rejected: u64,
    pub staged: u64,
}

const FIELDS: &str = "parent_native_session_id,rollout_id,launch_call_id,launch_operation_index,
    child_native_session_id,completion_call_id,completion_operation_index,process_session_id,
    launch_offset,completion_offset,launch_ordinal,completion_ordinal";

fn candidate_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LaunchCandidate> {
    Ok(LaunchCandidate {
        key: LaunchKey {
            parent_native_session_id: row.get(0)?,
            rollout_id: row.get(1)?,
            launch_call_id: row.get(2)?,
            launch_operation_index: row.get(3)?,
        },
        child_native_session_id: row.get(4)?,
        acknowledgment_call_id: row.get(5)?,
        acknowledgment_operation_index: row.get(6)?,
        process_session_id: row.get(7)?,
        launch_offset: row.get(8)?,
        acknowledgment_offset: row.get(9)?,
        launch_ordinal: row.get(10)?,
        acknowledgment_ordinal: row.get(11)?,
    })
}

fn row_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CandidateRow> {
    Ok(CandidateRow {
        candidate: candidate_from_row(row)?,
        source_revision: row.get(12)?,
        source_verdict: row.get(13)?,
        child_state: row.get(14)?,
    })
}

fn group(connection: &Connection, parent: &str) -> Result<Option<GroupRecord>> {
    Ok(connection
        .query_row(
            "SELECT revision,allocated,status,validator_version FROM claude_launch_groups
             WHERE parent_native_session_id=?1",
            [parent],
            |row| {
                Ok(GroupRecord {
                    revision: row.get(0)?,
                    allocated: row.get(1)?,
                    status: row.get(2)?,
                    validator_version: row.get(3)?,
                })
            },
        )
        .optional()?)
}

fn members(connection: &Connection, parent: &str) -> Result<Vec<MemberRecord>> {
    Ok(connection
        .prepare(
            "SELECT rollout_id,segment_device,segment_inode,segment_length,segment_mtime_ns,
                    segment_ctime_ns,scanned_length,status
             FROM claude_launch_group_members WHERE parent_native_session_id=?1
             ORDER BY rollout_id",
        )?
        .query_map([parent], |row| {
            Ok(MemberRecord {
                rollout_id: row.get(0)?,
                generation: SegmentGeneration {
                    device: row.get(1)?,
                    inode: row.get(2)?,
                    length: row.get(3)?,
                    mtime_ns: row.get(4)?,
                    ctime_ns: row.get(5)?,
                },
                scanned_length: row.get(6)?,
                valid: row.get::<_, String>(7)? == "valid",
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Bounded structural identifiers the tables require; anything else is
/// refused before a statement runs.
fn check_candidate(candidate: &LaunchCandidate) -> Result<()> {
    let uuid = |value: &str| {
        value.len() == 36
            && value
                .bytes()
                .all(|byte| byte == b'-' || byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    let token = |value: &str| {
        !value.is_empty() && value.len() <= 256 && value.bytes().all(|byte| byte.is_ascii_graphic())
    };
    let ordinals = match (candidate.launch_ordinal, candidate.acknowledgment_ordinal) {
        (None, None) => true,
        (Some(launch), Some(acknowledgment)) => launch >= 0 && acknowledgment > launch,
        _ => false,
    };
    let ok = uuid(&candidate.key.parent_native_session_id)
        && uuid(&candidate.key.rollout_id)
        && uuid(&candidate.child_native_session_id)
        && token(&candidate.key.launch_call_id)
        && token(&candidate.acknowledgment_call_id)
        && candidate
            .process_session_id
            .as_deref()
            .is_none_or(|handle| {
                !handle.is_empty()
                    && handle.len() <= 32
                    && handle.bytes().all(|b| b.is_ascii_digit())
            })
        && candidate.launch_offset >= 0
        && candidate.acknowledgment_offset > candidate.launch_offset
        && ordinals;
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidInput(
            "a launch candidate needs bounded structural fields",
        ))
    }
}

impl Store {
    /// A thread's published validation, if any.
    pub fn claude_launch_group(&self, parent: &str) -> Result<Option<GroupRecord>> {
        group(&self.connection, parent)
    }

    /// The member files of a thread's published validation, by rollout.
    pub fn claude_launch_members(&self, parent: &str) -> Result<Vec<MemberRecord>> {
        members(&self.connection, parent)
    }

    /// Start a validation attempt: the thread becomes `pending` (so nothing
    /// of it is linked until a validation is published), takes a new revision
    /// number never given before, and loses whatever an abandoned attempt
    /// staged. Returns the published revision it starts from and its own.
    pub fn begin_claude_launch_validation(
        &mut self,
        parent: &str,
        validator_version: u32,
    ) -> Result<Allocation> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = group(&transaction, parent)?;
        let expected = current.as_ref().and_then(|group| group.revision);
        let allocated = current.as_ref().map_or(0, |group| group.allocated) + 1;
        transaction.execute(
            "INSERT INTO claude_launch_groups(parent_native_session_id,revision,allocated,status,
                 validator_version) VALUES (?1,?2,?3,'pending',?4)
             ON CONFLICT(parent_native_session_id) DO UPDATE SET allocated=excluded.allocated,
                 status='pending',validator_version=excluded.validator_version",
            params![parent, expected, allocated, validator_version],
        )?;
        transaction.execute(
            "DELETE FROM claude_launch_staged_candidates WHERE parent_native_session_id=?1",
            [parent],
        )?;
        transaction.commit()?;
        Ok(Allocation {
            expected,
            allocated,
        })
    }

    /// Stage one chunk of an attempt's launches. Returns `false`, writing
    /// nothing, when the attempt is no longer the thread's latest.
    pub fn stage_claude_launch_candidates(
        &mut self,
        parent: &str,
        allocated: i64,
        candidates: &[LaunchCandidate],
    ) -> Result<bool> {
        if candidates.len() > MAX_CANDIDATE_ROWS {
            return Err(Error::InvalidInput(
                "too many launch candidates in one chunk",
            ));
        }
        for candidate in candidates {
            check_candidate(candidate)?;
            if candidate.key.parent_native_session_id != parent {
                return Err(Error::InvalidInput(
                    "a staged launch belongs to another thread",
                ));
            }
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if group(&transaction, parent)?.map(|group| group.allocated) != Some(allocated) {
            return Ok(false);
        }
        for c in candidates {
            transaction.execute(
                &format!(
                    "INSERT INTO claude_launch_staged_candidates(revision,{FIELDS})
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)"
                ),
                params![
                    allocated,
                    c.key.parent_native_session_id,
                    c.key.rollout_id,
                    c.key.launch_call_id,
                    c.key.launch_operation_index,
                    c.child_native_session_id,
                    c.acknowledgment_call_id,
                    c.acknowledgment_operation_index,
                    c.process_session_id,
                    c.launch_offset,
                    c.acknowledgment_offset,
                    c.launch_ordinal,
                    c.acknowledgment_ordinal,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    /// Publish an attempt at once: its member set, its staged launches and
    /// its verdict, only if the thread still holds `allocation.expected` as
    /// its published revision, `allocation.allocated` as its latest attempt,
    /// and exactly `staged` staged launches. A launch keeps `linked` only
    /// when every structural field equals the published linked row;
    /// otherwise it waits to be checked again, so a changed proof meets the
    /// store's conflict checks. Returns whether it was published.
    pub fn publish_claude_launch_validation(
        &mut self,
        parent: &str,
        allocation: Allocation,
        status: GroupStatus,
        members_read: &[MemberRecord],
        staged: usize,
    ) -> Result<bool> {
        if status == GroupStatus::Pending {
            return Err(Error::InvalidInput(
                "a published validation is valid or invalid",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(current) = group(&transaction, parent)? else {
            return Ok(false);
        };
        let count: i64 = transaction.query_row(
            "SELECT count(*) FROM claude_launch_staged_candidates
             WHERE parent_native_session_id=?1 AND revision=?2",
            params![parent, allocation.allocated],
            |row| row.get(0),
        )?;
        if current.revision != allocation.expected
            || current.allocated != allocation.allocated
            || usize::try_from(count).ok() != Some(staged)
            || (status == GroupStatus::Invalid && staged != 0)
        {
            return Ok(false);
        }
        let revision = allocation.allocated;
        transaction.execute(
            "DELETE FROM claude_launch_group_members WHERE parent_native_session_id=?1",
            [parent],
        )?;
        for member in members_read {
            let g = &member.generation;
            transaction.execute(
                "INSERT INTO claude_launch_group_members(parent_native_session_id,rollout_id,
                     revision,segment_device,segment_inode,segment_length,segment_mtime_ns,
                     segment_ctime_ns,scanned_length,status)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    parent,
                    member.rollout_id,
                    revision,
                    g.device,
                    g.inode,
                    g.length,
                    g.mtime_ns,
                    g.ctime_ns,
                    member.scanned_length,
                    if member.valid { "valid" } else { "invalid" },
                ],
            )?;
        }
        // Staged launches replace the published ones; `linked` is kept only
        // for an identical structural row.
        transaction.execute(
            &format!(
                "CREATE TEMP TABLE IF NOT EXISTS claude_launch_publish AS
                 SELECT {FIELDS},0 AS linked FROM claude_launch_candidates WHERE 0"
            ),
            [],
        )?;
        transaction.execute("DELETE FROM claude_launch_publish", [])?;
        transaction.execute(
            &format!(
                "INSERT INTO claude_launch_publish SELECT {FIELDS},
                     EXISTS(SELECT 1 FROM claude_launch_candidates c
                        WHERE c.parent_native_session_id=s.parent_native_session_id
                          AND c.rollout_id=s.rollout_id AND c.launch_call_id=s.launch_call_id
                          AND c.launch_operation_index=s.launch_operation_index
                          AND c.child_native_session_id=s.child_native_session_id
                          AND c.completion_call_id=s.completion_call_id
                          AND c.completion_operation_index=s.completion_operation_index
                          AND c.process_session_id IS s.process_session_id
                          AND c.launch_offset=s.launch_offset
                          AND c.completion_offset=s.completion_offset
                          AND c.launch_ordinal IS s.launch_ordinal
                          AND c.completion_ordinal IS s.completion_ordinal
                          AND c.child_state='linked' AND c.source_verdict='valid')
                 FROM claude_launch_staged_candidates s
                 WHERE s.parent_native_session_id=?1 AND s.revision=?2"
            ),
            params![parent, revision],
        )?;
        transaction.execute(
            "DELETE FROM claude_launch_candidates WHERE parent_native_session_id=?1",
            [parent],
        )?;
        transaction.execute(
            &format!(
                "INSERT INTO claude_launch_candidates({FIELDS},source_revision,source_verdict,
                     child_state)
                 SELECT {FIELDS},?1,'valid',CASE WHEN linked THEN 'linked' ELSE 'waiting' END
                 FROM claude_launch_publish"
            ),
            [revision],
        )?;
        transaction.execute("DELETE FROM claude_launch_publish", [])?;
        transaction.execute(
            "DELETE FROM claude_launch_staged_candidates WHERE parent_native_session_id=?1",
            [parent],
        )?;
        transaction.execute(
            "UPDATE claude_launch_groups SET revision=?2,status=?3 WHERE parent_native_session_id=?1",
            params![parent, revision, status],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    /// A page of a thread's published launches whose child is still to be
    /// looked at (`waiting` or `retry`, parent verdict `valid`, the current
    /// revision of a `valid` thread: none while pending), after `after` in
    /// key order.
    pub fn claude_launch_open_candidates(
        &self,
        parent: &str,
        after: Option<&LaunchKey>,
        limit: usize,
    ) -> Result<Vec<CandidateRow>> {
        let limit = i64::try_from(limit.min(MAX_CANDIDATE_ROWS)).unwrap_or(0);
        let (rollout, call, op) = after.map_or((None, None, None), |key| {
            (
                Some(key.rollout_id.as_str()),
                Some(key.launch_call_id.as_str()),
                Some(key.launch_operation_index),
            )
        });
        Ok(self
            .connection
            .prepare(&format!(
                "SELECT {FIELDS},source_revision,source_verdict,child_state
                 FROM claude_launch_candidates c
                 WHERE parent_native_session_id=?1 AND child_state IN ('waiting','retry')
                   AND source_verdict='valid'
                   AND source_revision=(SELECT revision FROM claude_launch_groups g
                       WHERE g.parent_native_session_id=c.parent_native_session_id
                         AND g.status='valid')
                   AND (?2 IS NULL OR (rollout_id,launch_call_id,launch_operation_index)
                        > (?2,?3,?4))
                 ORDER BY rollout_id,launch_call_id,launch_operation_index LIMIT ?5"
            ))?
            .query_map(params![parent, rollout, call, op, limit], row_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// A page of the launches naming the Claude session `child`, after
    /// `after` in key order, whatever their state.
    pub fn claude_launch_candidates_for_child(
        &self,
        child: &str,
        after: Option<&LaunchKey>,
        limit: usize,
    ) -> Result<Vec<CandidateRow>> {
        let limit = i64::try_from(limit.min(MAX_CANDIDATE_ROWS)).unwrap_or(0);
        let (parent, rollout, call, op) = after.map_or((None, None, None, None), |key| {
            (
                Some(key.parent_native_session_id.as_str()),
                Some(key.rollout_id.as_str()),
                Some(key.launch_call_id.as_str()),
                Some(key.launch_operation_index),
            )
        });
        Ok(self
            .connection
            .prepare(&format!(
                "SELECT {FIELDS},source_revision,source_verdict,child_state
                 FROM claude_launch_candidates WHERE child_native_session_id=?1
                   AND (?2 IS NULL OR (parent_native_session_id,rollout_id,launch_call_id,
                        launch_operation_index) > (?2,?3,?4,?5))
                 ORDER BY parent_native_session_id,rollout_id,launch_call_id,
                     launch_operation_index LIMIT ?6"
            ))?
            .query_map(
                params![child, parent, rollout, call, op, limit],
                row_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// A page of launches waiting for a timed retry, after `after`.
    pub fn claude_launch_retry_candidates(
        &self,
        after: Option<&LaunchKey>,
        limit: usize,
    ) -> Result<Vec<LaunchKey>> {
        let limit = i64::try_from(limit.min(MAX_CANDIDATE_ROWS)).unwrap_or(0);
        let (parent, rollout, call, op) = after.map_or((None, None, None, None), |key| {
            (
                Some(key.parent_native_session_id.as_str()),
                Some(key.rollout_id.as_str()),
                Some(key.launch_call_id.as_str()),
                Some(key.launch_operation_index),
            )
        });
        Ok(self
            .connection
            .prepare(
                "SELECT parent_native_session_id,rollout_id,launch_call_id,launch_operation_index
                 FROM claude_launch_candidates WHERE child_state='retry'
                   AND (?1 IS NULL OR (parent_native_session_id,rollout_id,launch_call_id,
                        launch_operation_index) > (?1,?2,?3,?4))
                 ORDER BY parent_native_session_id,rollout_id,launch_call_id,
                     launch_operation_index LIMIT ?5",
            )?
            .query_map(params![parent, rollout, call, op, limit], |row| {
                Ok(LaunchKey {
                    parent_native_session_id: row.get(0)?,
                    rollout_id: row.get(1)?,
                    launch_call_id: row.get(2)?,
                    launch_operation_index: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Set one published launch's child state, only if it is still the
    /// launch that was looked at: same structural fields and revision, parent
    /// verdict `valid`, and not linked. Returns whether it changed.
    pub fn set_claude_launch_child_state(
        &mut self,
        row: &CandidateRow,
        state: ChildState,
    ) -> Result<bool> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !same_row(&transaction, row)? {
            return Ok(false);
        }
        let k = &row.candidate.key;
        let changed = transaction.execute(
            "UPDATE claude_launch_candidates SET child_state=?5
             WHERE parent_native_session_id=?1 AND rollout_id=?2 AND launch_call_id=?3
               AND launch_operation_index=?4 AND child_state<>'linked' AND child_state<>?5",
            params![
                k.parent_native_session_id,
                k.rollout_id,
                k.launch_call_id,
                k.launch_operation_index,
                state
            ],
        )? > 0;
        transaction.commit()?;
        Ok(changed)
    }

    /// Record that a published launch's recorded lines no longer read as that
    /// launch; only a new validation looks at it again.
    pub fn reject_claude_launch_source(&mut self, row: &CandidateRow) -> Result<bool> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !same_row(&transaction, row)? {
            return Ok(false);
        }
        let k = &row.candidate.key;
        let changed = transaction.execute(
            "UPDATE claude_launch_candidates SET source_verdict='rejected'
             WHERE parent_native_session_id=?1 AND rollout_id=?2 AND launch_call_id=?3
               AND launch_operation_index=?4 AND child_state<>'linked'",
            params![
                k.parent_native_session_id,
                k.rollout_id,
                k.launch_call_id,
                k.launch_operation_index
            ],
        )? > 0;
        transaction.commit()?;
        Ok(changed)
    }

    /// Look again at the child-side rejections of launches naming `child`,
    /// now that it was imported again: only launches of a thread's current
    /// valid revision whose parent verdict is `valid`. Parent-side
    /// rejections are left to a new validation. Returns how many reopened.
    pub fn reopen_claude_launch_child(&mut self, child: &str) -> Result<usize> {
        Ok(self.connection.execute(
            "UPDATE claude_launch_candidates SET child_state='waiting'
             WHERE child_native_session_id=?1 AND child_state='rejected'
               AND source_verdict='valid'
               AND source_revision=(SELECT revision FROM claude_launch_groups g
                   WHERE g.parent_native_session_id=claude_launch_candidates.parent_native_session_id
                     AND g.status='valid')",
            [child],
        )?)
    }

    /// Whether any launch waits for a timed retry.
    pub fn claude_launch_retry_pending(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM claude_launch_candidates WHERE child_state='retry')",
            [],
            |row| row.get(0),
        )?)
    }

    /// Write one launch relation only if, in the same transaction, the
    /// thread's published validation is still `valid` at the resolution's
    /// revision with exactly the member generations it checked, and the
    /// published launch still has exactly the structural fields, revision,
    /// parent verdict and an open child state it checked. Then the reviewed
    /// proof checks apply, and the launch becomes `linked`. `None` when the
    /// guard failed and nothing was written.
    pub fn record_claude_launch_guarded(
        &mut self,
        proof: &ClaudeLaunchCreationProof,
        row: &CandidateRow,
        members_checked: &[MemberRecord],
        recorded_at: i64,
    ) -> Result<Option<CreationReport>> {
        validate_launch(proof)?;
        let c = &row.candidate;
        if proof.child_native_session_id != c.child_native_session_id
            || proof.parent_native_session_id != c.key.parent_native_session_id
            || proof.segment_rollout_id != c.key.rollout_id
            || proof.launch_call_id != c.key.launch_call_id
            || proof.launch_operation_index != c.key.launch_operation_index
            || proof.acknowledgment_call_id != c.acknowledgment_call_id
            || proof.acknowledgment_operation_index != c.acknowledgment_operation_index
            || proof.process_session_id != c.process_session_id
            || proof.launch_ordinal != c.launch_ordinal
            || proof.acknowledgment_ordinal != c.acknowledgment_ordinal
        {
            return Err(Error::InvalidInput(
                "a launch proof must be its candidate's",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = group(&transaction, &c.key.parent_native_session_id)?;
        let valid_now = current.is_some_and(|group| {
            group.status == GroupStatus::Valid && group.revision == Some(row.source_revision)
        });
        let members_now = members(&transaction, &c.key.parent_native_session_id)?;
        let mut checked: Vec<&MemberRecord> = members_checked.iter().collect();
        checked.sort_by(|a, b| a.rollout_id.cmp(&b.rollout_id));
        let same_members = members_now.len() == checked.len()
            && members_now
                .iter()
                .zip(&checked)
                .all(|(now, then)| now == *then);
        if !valid_now || !same_members || !same_row(&transaction, row)? {
            return Ok(None);
        }
        let (disposition, changed) = apply_launch(&transaction, proof, recorded_at)?;
        let k = &c.key;
        // An abstention leaves the launch open for its caller to decide.
        if !matches!(
            disposition,
            crate::creation::CreationDisposition::Abstained(_)
        ) {
            transaction.execute(
                "UPDATE claude_launch_candidates SET child_state='linked'
             WHERE parent_native_session_id=?1 AND rollout_id=?2 AND launch_call_id=?3
               AND launch_operation_index=?4",
                params![
                    k.parent_native_session_id,
                    k.rollout_id,
                    k.launch_call_id,
                    k.launch_operation_index
                ],
            )?;
        }
        transaction.commit()?;
        Ok(Some(CreationReport {
            dispositions: vec![disposition],
            changed: usize::from(changed),
        }))
    }

    /// Metadata-only counts of the launch relations and their state.
    pub fn claude_launch_summary(&self) -> Result<LaunchSummary> {
        let count = |sql: &str| -> Result<u64> {
            let value: i64 = self.connection.query_row(sql, [], |row| row.get(0))?;
            Ok(u64::try_from(value).unwrap_or(0))
        };
        let state = |name: &str| {
            count(&format!(
                "SELECT count(*) FROM claude_launch_candidates WHERE child_state='{name}'"
            ))
        };
        let status = |name: &str| {
            count(&format!(
                "SELECT count(*) FROM claude_launch_groups WHERE status='{name}'"
            ))
        };
        Ok(LaunchSummary {
            relations_accepted: count(
                "SELECT count(*) FROM session_creation_relations
                 WHERE evidence_kind='codex_claude_launch' AND state='accepted'",
            )?,
            relations_conflicted: count(
                "SELECT count(*) FROM session_creation_relations
                 WHERE evidence_kind='codex_claude_launch' AND state='conflicted'",
            )?,
            groups_valid: status("valid")?,
            groups_invalid: status("invalid")?,
            groups_pending: status("pending")?,
            members: count("SELECT count(*) FROM claude_launch_group_members")?,
            members_invalid: count(
                "SELECT count(*) FROM claude_launch_group_members WHERE status='invalid'",
            )?,
            bytes_covered: count(
                "SELECT coalesce(sum(scanned_length),0) FROM claude_launch_group_members",
            )?,
            candidates_waiting: state("waiting")?,
            candidates_retry: state("retry")?,
            candidates_linked: state("linked")?,
            candidates_rejected: state("rejected")?,
            candidates_source_rejected: count(
                "SELECT count(*) FROM claude_launch_candidates WHERE source_verdict='rejected'",
            )?,
            staged: count("SELECT count(*) FROM claude_launch_staged_candidates")?,
        })
    }
}

/// Whether the published launch still has exactly `row`'s structural
/// fields, revision and parent verdict `valid`, with an open child state.
fn same_row(connection: &Connection, row: &CandidateRow) -> Result<bool> {
    let k = &row.candidate.key;
    let stored = connection
        .query_row(
            &format!(
                "SELECT {FIELDS},source_revision,source_verdict,child_state
                 FROM claude_launch_candidates WHERE parent_native_session_id=?1 AND rollout_id=?2
                   AND launch_call_id=?3 AND launch_operation_index=?4"
            ),
            params![
                k.parent_native_session_id,
                k.rollout_id,
                k.launch_call_id,
                k.launch_operation_index
            ],
            row_from_row,
        )
        .optional()?;
    Ok(stored.is_some_and(|stored| {
        stored.candidate == row.candidate
            && stored.source_revision == row.source_revision
            && stored.source_verdict == SourceVerdict::Valid
            && matches!(stored.child_state, ChildState::Waiting | ChildState::Retry)
    }))
}
