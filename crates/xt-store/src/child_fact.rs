//! That an indexed user session was created by an agent, apart from which
//! agent that was: child facts.
//!
//! A detector records a child fact as soon as it has proved the child, before
//! it looks for the parent, and whether or not a parent is ever found. A
//! child fact never names a parent and never stands in for one: parents stay
//! in [`crate::creation`] relations and saved Human origins, which this module
//! does not touch. It feeds the known-child bit of the Sessions and Dashboard
//! reads ([`crate::session_list`]) and nothing else: no metric, span, usage,
//! Human input, search or membership query reads it.
//!
//! Each fact is bound to the exact canonical child, its host and native
//! identity — exactly one indexed user session holds it — and to where the
//! evidence lies: the child's own Codex header, or another session's launch
//! (that session, its history file for a Codex thread, the launch call and
//! operation) and the child's own first input. Nothing else is kept: no
//! prompt, command, answer, path, title or digest.
//!
//! Independent evidence for one child is kept side by side; none contradicts
//! another, and two possible creators only make the parent uncertain. One
//! launch names at most one child with one first input: a launch claimed for
//! a second child, or for its child with another first input, withholds every
//! fact resting on that launch, for good, and only those. Replaying a stored
//! fact changes nothing.

use crate::{
    Error, Host, Result, Store,
    creation::{
        CLAUDE_LAUNCH_CREATE_VERSION, CODEX_CLI_LAUNCH_VERSION, CODEX_THREAD_SPAWN_VERSION,
        CreationAbstention, child_abstention, first_input, token,
    },
    model::text_enum,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

/// Facts accepted by one call. A caller with more splits them into batches.
pub const MAX_CHILD_FACTS: usize = 1_000;

/// Version of the typed Guardian header reading a Guardian child fact rests on.
pub const CODEX_GUARDIAN_CHILD_VERSION: u32 = 1;

/// Version of the Claude `Bash` launch validation a child fact rests on.
/// Version 2 reads launch options through the shared Claude option map; its
/// answer checks are unchanged. Stored version 1 facts stand as they are.
pub const CLAUDE_BASH_CHILD_VERSION: u32 = 2;

// Where a child fact's evidence lies. `codex_thread_spawn` and
// `codex_guardian`: the Codex child's own opening `session_meta` says, by its
// typed `source`, that it is a spawned or Guardian subagent.
// `codex_claude_launch`: a Codex thread's own recorded `exec` launch of a
// create-mode Claude command named the child with `--session-id`.
// `claude_bash_launch`: a Claude session's own `Bash` call launched a Claude
// print session whose child was found by its first input and whole answer.
// `codex_cli_launch`: a Codex thread's own recorded `exec` launch of a fresh
// `codex exec --json` command, whose own first process result named the
// Codex child.
text_enum!(ChildEvidence {
    CodexThreadSpawn => "codex_thread_spawn",
    CodexGuardian => "codex_guardian",
    CodexClaudeLaunch => "codex_claude_launch",
    ClaudeBashLaunch => "claude_bash_launch",
    CodexCliLaunch => "codex_cli_launch",
});

impl ChildEvidence {
    /// The one evidence version this store accepts for the kind.
    pub fn version(self) -> u32 {
        match self {
            Self::CodexThreadSpawn => CODEX_THREAD_SPAWN_VERSION,
            Self::CodexGuardian => CODEX_GUARDIAN_CHILD_VERSION,
            Self::CodexClaudeLaunch => CLAUDE_LAUNCH_CREATE_VERSION,
            Self::ClaudeBashLaunch => CLAUDE_BASH_CHILD_VERSION,
            Self::CodexCliLaunch => CODEX_CLI_LAUNCH_VERSION,
        }
    }

    fn typed(self) -> bool {
        matches!(self, Self::CodexThreadSpawn | Self::CodexGuardian)
    }
}

/// One piece of evidence that a session was created by an agent. Every field
/// is structural; none is transcript content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildFact {
    pub child_session_id: String,
    pub child_host: Host,
    pub child_native_session_id: String,
    pub evidence_kind: ChildEvidence,
    pub evidence_version: u32,
    /// The session the evidence lies in: the child itself for a typed
    /// header, the launching session for a launch.
    pub source_native_session_id: String,
    /// The Codex history file a `codex_claude_launch` or `codex_cli_launch`
    /// lies in.
    pub source_rollout_id: Option<String>,
    /// A launch's call and its operation (or position) within it.
    pub launch_call_id: Option<String>,
    pub launch_operation_index: Option<u32>,
    /// A launched child's own first input record.
    pub first_record_uuid: Option<String>,
}

impl ChildFact {
    /// A Codex child's own typed header: a thread spawn or a Guardian.
    pub fn typed(child_session_id: &str, native: &str, kind: ChildEvidence) -> Self {
        Self {
            child_session_id: child_session_id.to_owned(),
            child_host: Host::Codex,
            child_native_session_id: native.to_owned(),
            evidence_kind: kind,
            evidence_version: kind.version(),
            source_native_session_id: native.to_owned(),
            source_rollout_id: None,
            launch_call_id: None,
            launch_operation_index: None,
            first_record_uuid: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildFactDisposition {
    /// Newly stored.
    Recorded,
    /// This fact was already stored; nothing changed.
    AlreadyRecorded,
    /// Its launch names another child or another first input, now or before:
    /// every fact on that launch is withheld.
    Withheld,
    /// The store cannot tie it to one child; nothing was stored.
    Abstained(CreationAbstention),
}

/// One disposition per supplied fact, in input order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildFactReport {
    pub dispositions: Vec<ChildFactDisposition>,
    /// How many facts changed what a read shows: a fact newly accepted, or
    /// accepted facts first withheld.
    pub changed: usize,
}

/// Metadata-only counts of stored child facts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildFactSummary {
    /// `(kind, accepted, withheld)` for every kind.
    pub kinds: Vec<(String, u64, u64)>,
    /// Distinct children with at least one accepted fact.
    pub children: u64,
}

const FIELDS: &str = "child_session_id,child_host,child_native_session_id,evidence_kind,
    evidence_version,source_native_session_id,source_rollout_id,launch_call_id,
    launch_operation_index,first_record_uuid";

/// The rows of one launch, as `(child, first input, accepted)`.
const LAUNCH_ROWS: &str = "SELECT child_session_id,first_record_uuid,state='accepted'
    FROM session_child_facts WHERE evidence_kind=?1 AND source_native_session_id=?2
      AND ifnull(source_rollout_id,'')=?3 AND launch_call_id=?4 AND launch_operation_index=?5";

impl Store {
    /// Apply child facts in one immediate transaction, in order.
    ///
    /// A fact whose fields do not fit its kind or version, or too many facts,
    /// reject the whole call before anything is written. Otherwise every fact
    /// receives a disposition and what was stored commits together.
    pub fn record_child_facts(
        &mut self,
        facts: &[ChildFact],
        recorded_at: i64,
    ) -> Result<ChildFactReport> {
        if facts.len() > MAX_CHILD_FACTS {
            return Err(Error::InvalidInput("too many child facts for one call"));
        }
        for fact in facts {
            validate(fact)?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut dispositions = Vec::with_capacity(facts.len());
        let mut changed = 0;
        for fact in facts {
            let (disposition, change) = apply(&transaction, fact, recorded_at)?;
            dispositions.push(disposition);
            changed += change;
        }
        transaction.commit()?;
        Ok(ChildFactReport {
            dispositions,
            changed,
        })
    }

    /// Withhold every accepted fact resting on one launch, for a detector
    /// that can no longer tell which child the launch created. Returns how
    /// many were withheld.
    pub fn withhold_child_launch(
        &mut self,
        kind: ChildEvidence,
        source_native_session_id: &str,
        source_rollout_id: Option<&str>,
        launch_call_id: &str,
        launch_operation_index: u32,
    ) -> Result<usize> {
        if kind.typed() {
            return Err(Error::InvalidInput("a typed header names no launch"));
        }
        Ok(self.connection.execute(
            "UPDATE session_child_facts SET state='withheld'
             WHERE evidence_kind=?1 AND source_native_session_id=?2
               AND ifnull(source_rollout_id,'')=?3 AND launch_call_id=?4
               AND launch_operation_index=?5 AND state='accepted'",
            params![
                kind,
                source_native_session_id,
                source_rollout_id.unwrap_or(""),
                launch_call_id,
                launch_operation_index
            ],
        )?)
    }

    /// Every stored fact resting on one launch: `(child, accepted)`.
    pub fn child_launch_facts(
        &self,
        kind: ChildEvidence,
        source_native_session_id: &str,
        source_rollout_id: Option<&str>,
        launch_call_id: &str,
        launch_operation_index: u32,
    ) -> Result<Vec<(String, bool)>> {
        Ok(self
            .connection
            .prepare(LAUNCH_ROWS)?
            .query_map(
                params![
                    kind,
                    source_native_session_id,
                    source_rollout_id.unwrap_or(""),
                    launch_call_id,
                    launch_operation_index
                ],
                |row| Ok((row.get(0)?, row.get(2)?)),
            )?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Every stored fact of one child, and whether it is accepted.
    pub fn child_facts(&self, child_session_id: &str) -> Result<Vec<(ChildFact, bool)>> {
        Ok(self
            .connection
            .prepare(&format!(
                "SELECT {FIELDS},state='accepted' FROM session_child_facts
                 WHERE child_session_id=?1 ORDER BY rowid"
            ))?
            .query_map([child_session_id], |row| {
                Ok((
                    ChildFact {
                        child_session_id: row.get(0)?,
                        child_host: row.get(1)?,
                        child_native_session_id: row.get(2)?,
                        evidence_kind: row.get(3)?,
                        evidence_version: row.get(4)?,
                        source_native_session_id: row.get(5)?,
                        source_rollout_id: row.get(6)?,
                        launch_call_id: row.get(7)?,
                        launch_operation_index: row.get(8)?,
                        first_record_uuid: row.get(9)?,
                    },
                    row.get(10)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Indexed Claude user sessions whose records hold a `Bash` tool call,
    /// after `after` in canonical identifier order, at most `limit`: the
    /// sessions that may have launched a Claude child from their own shell.
    /// `(canonical, native)` pairs. Index metadata only.
    pub fn claude_bash_callers(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        Ok(self
            .connection
            .prepare(
                "SELECT s.session_id,s.native_session_id FROM sessions s
                 WHERE s.host='claude' AND s.kind='user' AND s.native_session_id IS NOT NULL
                   AND (?1 IS NULL OR s.session_id>?1)
                   AND EXISTS(SELECT 1 FROM tool_uses t
                       WHERE t.session_id=s.session_id AND t.name='Bash')
                 ORDER BY s.session_id LIMIT ?2",
            )?
            .query_map(params![after, limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Indexed Claude user sessions with a `Bash` tool call recorded between
    /// `from_ms` and `to_ms`, at most `limit`. `(canonical, native)` pairs.
    pub fn claude_bash_callers_between(
        &self,
        from_ms: i64,
        to_ms: i64,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        Ok(self
            .connection
            .prepare(
                "SELECT DISTINCT s.session_id,s.native_session_id FROM records r
                 JOIN tool_uses t ON t.uuid=r.uuid AND t.session_id=r.session_id
                 JOIN sessions s ON s.session_id=r.session_id
                 WHERE r.ts_ms BETWEEN ?1 AND ?2 AND t.name='Bash'
                   AND s.host='claude' AND s.kind='user' AND s.native_session_id IS NOT NULL
                 ORDER BY s.session_id LIMIT ?3",
            )?
            .query_map(params![from_ms, to_ms, limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Indexed Claude user sessions with any record dated between `from_ms`
    /// and `to_ms`, whatever its tool calls, at most `limit`: every session
    /// that was being written then, so a caller whose calls the index does
    /// not hold yet is not passed over. `(canonical, native)` pairs.
    pub fn claude_sessions_active_between(
        &self,
        from_ms: i64,
        to_ms: i64,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        Ok(self
            .connection
            .prepare(
                "SELECT DISTINCT s.session_id,s.native_session_id FROM records r
                 JOIN sessions s ON s.session_id=r.session_id
                 WHERE r.ts_ms BETWEEN ?1 AND ?2
                   AND s.host='claude' AND s.kind='user' AND s.native_session_id IS NOT NULL
                 ORDER BY s.session_id LIMIT ?3",
            )?
            .query_map(params![from_ms, to_ms, limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Native identities, and recorded working directories, of indexed
    /// Claude user sessions whose earliest eligible input (a user record that
    /// is not meta, sidechain or a tool result) was recorded between
    /// `from_ms` and `to_ms`, at most `limit`.
    pub fn claude_sessions_born_between(
        &self,
        from_ms: i64,
        to_ms: i64,
        limit: usize,
    ) -> Result<Vec<(String, Option<String>)>> {
        Ok(self
            .connection
            .prepare(
                "SELECT DISTINCT s.native_session_id,s.cwd FROM records r
                 JOIN sessions s ON s.session_id=r.session_id
                 WHERE r.ts_ms BETWEEN ?1 AND ?2 AND r.type='user' AND r.role='user'
                   AND r.is_meta=0 AND r.is_sidechain=0 AND r.is_tool_result_carrier=0
                   AND s.host='claude' AND s.kind='user' AND s.native_session_id IS NOT NULL
                   AND NOT EXISTS(SELECT 1 FROM records q WHERE q.session_id=r.session_id
                       AND q.type='user' AND q.role='user' AND q.is_meta=0
                       AND q.is_sidechain=0 AND q.is_tool_result_carrier=0
                       AND (q.ts_ms IS NULL OR q.ts_ms<?1))
                 ORDER BY s.native_session_id LIMIT ?3",
            )?
            .query_map(params![from_ms, to_ms, limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// When the indexed Claude user session with this native identity
    /// recorded its earliest eligible input, if it is exactly one session.
    pub fn claude_first_input_ms(&self, native: &str) -> Result<Option<i64>> {
        let sessions =
            crate::creation::user_sessions_with_native(&self.connection, Host::Claude, native)?;
        let [session] = sessions.as_slice() else {
            return Ok(None);
        };
        Ok(self.connection.query_row(
            "SELECT min(ts_ms) FROM records WHERE session_id=?1 AND type='user'
               AND role='user' AND is_meta=0 AND is_sidechain=0 AND is_tool_result_carrier=0",
            [session],
            |row| row.get(0),
        )?)
    }

    /// The indexed session's own first eligible input, as the creation checks
    /// define it ([`crate::creation`]): its record identifier and time, when
    /// one timed user record with text, not meta, sidechain or a tool result,
    /// has no other such record, with text or without, untimed or at or
    /// before it. Index metadata only.
    pub fn first_input_record(&self, session_id: &str) -> Result<Option<(String, i64)>> {
        first_input_record(&self.connection, session_id)
    }
}

/// Metadata used by the native record transaction to notice a changed first
/// eligible input before its new rows become visible.
pub(crate) fn first_input_record(
    connection: &rusqlite::Connection,
    session_id: &str,
) -> Result<Option<(String, i64)>> {
    let earliest: Option<(String, i64)> = connection
        .query_row(
            "SELECT uuid,ts_ms FROM records WHERE session_id=?1 AND type='user'
                   AND role='user' AND is_meta=0 AND is_sidechain=0
                   AND is_tool_result_carrier=0 AND text_len>0 AND ts_ms IS NOT NULL
                 ORDER BY ts_ms,uuid LIMIT 1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match earliest {
        Some((uuid, at)) if first_input(connection, session_id, &uuid)? => Some((uuid, at)),
        _ => None,
    })
}

impl Store {
    /// Metadata-only counts of the stored facts.
    pub fn child_fact_summary(&self) -> Result<ChildFactSummary> {
        let kinds = self
            .connection
            .prepare(
                "SELECT evidence_kind,sum(state='accepted'),sum(state='withheld')
                 FROM session_child_facts GROUP BY evidence_kind ORDER BY evidence_kind",
            )?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?.max(0) as u64,
                    row.get::<_, i64>(2)?.max(0) as u64,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let children: i64 = self.connection.query_row(
            "SELECT count(DISTINCT child_session_id) FROM session_child_facts
             WHERE state='accepted'",
            [],
            |row| row.get(0),
        )?;
        Ok(ChildFactSummary {
            kinds,
            children: children.max(0) as u64,
        })
    }
}

/// Whether a fact's fields fit its kind and the version this store accepts.
pub(crate) fn validate(fact: &ChildFact) -> Result<()> {
    let invalid = |why| Err(Error::InvalidInput(why));
    for value in [
        &fact.child_session_id,
        &fact.child_native_session_id,
        &fact.source_native_session_id,
    ] {
        token(value)?;
    }
    for value in [
        &fact.source_rollout_id,
        &fact.launch_call_id,
        &fact.first_record_uuid,
    ]
    .into_iter()
    .flatten()
    {
        token(value)?;
    }
    if fact.evidence_version != fact.evidence_kind.version() {
        return invalid("a child fact requires its kind's known version");
    }
    let launch = (
        &fact.launch_call_id,
        fact.launch_operation_index,
        &fact.first_record_uuid,
    );
    let rollout = fact.source_rollout_id.as_deref().is_some_and(|rollout| {
        rollout.len() == 36
            && rollout
                .bytes()
                .all(|b| b == b'-' || b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    });
    let fits = match fact.evidence_kind {
        ChildEvidence::CodexThreadSpawn | ChildEvidence::CodexGuardian => {
            fact.child_host == Host::Codex
                && fact.source_native_session_id == fact.child_native_session_id
                && fact.source_rollout_id.is_none()
                && matches!(launch, (None, None, None))
        }
        ChildEvidence::CodexClaudeLaunch => {
            fact.child_host == Host::Claude
                && fact.source_native_session_id != fact.child_native_session_id
                && rollout
                && matches!(launch, (Some(_), Some(_), Some(_)))
        }
        ChildEvidence::CodexCliLaunch => {
            fact.child_host == Host::Codex
                && fact.source_native_session_id != fact.child_native_session_id
                && rollout
                && matches!(launch, (Some(_), Some(_), Some(_)))
        }
        ChildEvidence::ClaudeBashLaunch => {
            fact.child_host == Host::Claude
                && fact.source_native_session_id != fact.child_native_session_id
                && fact.source_rollout_id.is_none()
                && matches!(launch, (Some(_), Some(_), Some(_)))
        }
    };
    if !fits {
        return invalid("a child fact does not fit its evidence kind");
    }
    Ok(())
}

/// A fact's disposition, and how many facts applying it changed for a read.
pub(crate) fn apply(
    connection: &Connection,
    fact: &ChildFact,
    recorded_at: i64,
) -> Result<(ChildFactDisposition, usize)> {
    let abstain = |reason| Ok((ChildFactDisposition::Abstained(reason), 0));
    if let Some(reason) = child_abstention(
        connection,
        &fact.child_session_id,
        fact.child_host,
        &fact.child_native_session_id,
    )? {
        return abstain(reason);
    }
    if let Some(first) = &fact.first_record_uuid
        && !first_input(connection, &fact.child_session_id, first)?
    {
        return abstain(CreationAbstention::FirstInputMismatch);
    }
    // A launch names one child with one first input. Any other claim on it,
    // now or stored, withholds every fact resting on it.
    let mut withheld_now = 0;
    let mut disputed = false;
    if let (Some(call), Some(index)) = (&fact.launch_call_id, fact.launch_operation_index) {
        let rows = connection
            .prepare(LAUNCH_ROWS)?
            .query_map(
                params![
                    fact.evidence_kind,
                    fact.source_native_session_id,
                    fact.source_rollout_id.as_deref().unwrap_or(""),
                    call,
                    index
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let other = rows.iter().any(|(child, first, _)| {
            *child != fact.child_session_id || *first != fact.first_record_uuid
        });
        disputed = other || rows.iter().any(|(_, _, accepted)| !accepted);
        if other {
            withheld_now = connection.execute(
                "UPDATE session_child_facts SET state='withheld'
                 WHERE evidence_kind=?1 AND source_native_session_id=?2
                   AND ifnull(source_rollout_id,'')=?3 AND launch_call_id=?4
                   AND launch_operation_index=?5 AND state='accepted'",
                params![
                    fact.evidence_kind,
                    fact.source_native_session_id,
                    fact.source_rollout_id.as_deref().unwrap_or(""),
                    call,
                    index
                ],
            )?;
        }
    }
    let stored: Option<bool> = connection
        .query_row(
            "SELECT state='accepted' FROM session_child_facts
             WHERE child_session_id=?1 AND evidence_kind=?2 AND source_native_session_id=?3
               AND ifnull(source_rollout_id,'')=?4 AND ifnull(launch_call_id,'')=?5
               AND ifnull(launch_operation_index,-1)=?6",
            params![
                fact.child_session_id,
                fact.evidence_kind,
                fact.source_native_session_id,
                fact.source_rollout_id.as_deref().unwrap_or(""),
                fact.launch_call_id.as_deref().unwrap_or(""),
                fact.launch_operation_index.map_or(-1, i64::from)
            ],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(accepted) = stored {
        return Ok(if accepted && !disputed {
            (ChildFactDisposition::AlreadyRecorded, 0)
        } else {
            (ChildFactDisposition::Withheld, withheld_now)
        });
    }
    connection.execute(
        &format!(
            "INSERT INTO session_child_facts({FIELDS},state,recorded_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"
        ),
        params![
            fact.child_session_id,
            fact.child_host,
            fact.child_native_session_id,
            fact.evidence_kind,
            fact.evidence_version,
            fact.source_native_session_id,
            fact.source_rollout_id,
            fact.launch_call_id,
            fact.launch_operation_index,
            fact.first_record_uuid,
            if disputed { "withheld" } else { "accepted" },
            recorded_at,
        ],
    )?;
    Ok(if disputed {
        (ChildFactDisposition::Withheld, withheld_now)
    } else {
        (ChildFactDisposition::Recorded, 1)
    })
}
