//! Whether the supported checks of who created an indexed session have
//! finished against what the index holds of it now: its display state.
//!
//! A session is `checking` from the commit that first indexes it until the
//! checks this version supports finish for it. Each session's row holds its
//! current check attempt and a key naming the inputs of its own checks as
//! last observed — its opening, identity and first input, as digests
//! ([`Store::observe_own_check`]). A reader that observes different inputs
//! moves the attempt on before it publishes anything read from them; one
//! that cannot read them moves it on too, since a failed comparison proves
//! nothing unchanged. An ordinary later message changes neither, so a checked
//! session stays checked. A launch naming exactly a session, newly published
//! or changed, moves that session's attempt on as well. A background pass
//! marks the checks finished for the attempt and key it captured, with the
//! current detector versions, only if the session still requires exactly
//! those ([`Store::settle_child_checks`]). A session indexed before this
//! table existed, a newer attempt, or other detector versions read as
//! `checking`.
//!
//! The three states are a display fact only. `child` is the positive known
//! child bit of [`crate::session_list`], unchanged and independent of this
//! table. `checked` says only that the supported checks finished without
//! child evidence: never that a person did. No metric, span,
//! usage, Human input, search, membership or ordering query reads it.
//!
//! Nothing is kept but identities, generations and one closed version label.

use crate::{Host, Result, Store};
use rusqlite::{Connection, params};

/// The versions of the detectors a finished check ran, as stored with it:
/// the check itself, the Codex header reading, the Codex history scan
/// ([`LAUNCH_VALIDATION_VERSION`]), the Claude and Codex launch proofs and
/// the Claude `Bash` launch validation. A check finished under any other
/// versions is a check still to run.
macro_rules! child_check_versions {
    () => {
        "check1.spawn2.scan10.claude6.cli1.bash2"
    };
}
pub(crate) use child_check_versions;

/// The detector versions every finished check is stored with today.
pub const DETECTOR_VERSIONS: &str = child_check_versions!();

/// The Codex history scan version (`CLAUDE_LAUNCH_VALIDATION_VERSION` of the
/// native reader) that [`DETECTOR_VERSIONS`] names. The reader asserts they
/// agree.
pub const LAUNCH_VALIDATION_VERSION: u32 = 10;

/// Sessions one [`Store::open_child_checks`] page holds at most.
pub const MAX_OPEN_CHECKS: usize = 512;

/// Whether stored positive evidence says another session created the session
/// aliased `s`: the known-child bit of [`crate::session_list::context`].
macro_rules! known_child_sql {
    () => {
        "(EXISTS(SELECT 1 FROM session_creation_relations c WHERE c.child_session_id=s.session_id
          AND c.child_host=s.host AND c.child_native_session_id=s.native_session_id
          AND c.state='accepted')
       OR EXISTS(SELECT 1 FROM human_session_origins o WHERE o.session_id=s.session_id
          AND o.host=s.host AND o.native_session_id=s.native_session_id AND o.conflicted=0)
       OR EXISTS(SELECT 1 FROM session_child_facts f WHERE f.child_session_id=s.session_id
          AND f.child_host=s.host AND f.child_native_session_id=s.native_session_id
          AND f.state='accepted'))"
    };
}
pub(crate) use known_child_sql;

/// Whether the session aliased `s` has finished its checks for the
/// generation it requires now, under today's detector versions.
macro_rules! checked_sql {
    () => {
        concat!(
            "EXISTS(SELECT 1 FROM session_child_checks k WHERE k.session_id=s.session_id
          AND k.completed_generation=k.required_generation
          AND k.detector_versions='",
            crate::child_check::child_check_versions!(),
            "')"
        )
    };
}
pub(crate) use checked_sql;

/// What a list may show of one session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildCheck {
    /// The supported checks have not finished for what the index holds of it
    /// now, or failed, or ran under other detector versions: not listed as a
    /// session of its own.
    Checking,
    /// The supported checks finished without child evidence. Not a claim
    /// that a person created it.
    Checked,
    /// Stored positive evidence says another session created it, whatever
    /// its checks say.
    Child,
}

/// One session whose checks are still to finish, as a background pass
/// captured it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenCheck {
    pub session_id: String,
    pub host: Host,
    pub native_session_id: Option<String>,
    /// The attempt a finished check must answer for.
    pub required: i64,
    /// Its own-check inputs as last observed, if they were.
    pub own_check_key: Option<String>,
    /// The start the host recorded, if any.
    pub started_at_ms: Option<i64>,
    pub cwd: Option<String>,
}

/// Sessions one invalidation names at most.
pub const MAX_INVALIDATIONS: usize = 512;

/// Inside the ingest transaction that commits a session's records: a session
/// committed for the first time starts checking, at its first attempt. A
/// later commit changes nothing here.
pub(crate) fn require(connection: &Connection, session_id: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO session_child_checks(session_id,required_generation) VALUES (?1,1)
         ON CONFLICT(session_id) DO NOTHING",
        params![session_id],
    )?;
    Ok(())
}

/// A well-formed own-check key: a closed prefix and a lowercase digest.
fn check_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.len() > 96
        || !key
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b == b':')
    {
        return Err(crate::Error::InvalidInput("invalid own-check key"));
    }
    Ok(())
}

/// The attempt a session requires now, with the own-check key it was
/// observed with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub required: i64,
    pub own_check_key: Option<String>,
}

impl Store {
    /// A bounded page of completed Claude user sessions whose own source
    /// needs a cheap presence/readability check during native reconciliation.
    pub fn completed_claude_checks(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        let mut statement = self.connection.prepare(
            "SELECT s.session_id,s.native_session_id FROM sessions s
             JOIN session_child_checks k ON k.session_id=s.session_id
             WHERE s.host='claude' AND s.kind='user' AND s.native_session_id IS NOT NULL
               AND k.completed_generation=k.required_generation
               AND k.detector_versions=?1 AND (?2 IS NULL OR s.session_id>?2)
             ORDER BY s.session_id LIMIT ?3",
        )?;
        Ok(statement
            .query_map(
                params![
                    DETECTOR_VERSIONS,
                    after,
                    i64::try_from(limit.min(512)).unwrap_or(512)
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Whether a public list currently treats this session's check as
    /// completed. A native importer compares such an old result before it
    /// publishes changed rows from the same conversation.
    pub fn completed_child_check(&self, session_id: &str) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_child_checks
             WHERE session_id=?1 AND completed_generation=required_generation
               AND detector_versions=?2)",
            params![session_id, DETECTOR_VERSIONS],
            |row| row.get(0),
        )?)
    }

    /// The attempt the session requires now, if it has a row.
    pub fn child_check_attempt(&self, session_id: &str) -> Result<Option<Attempt>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .connection
            .query_row(
                "SELECT required_generation,own_check_key FROM session_child_checks
                 WHERE session_id=?1",
                [session_id],
                |row| {
                    Ok(Attempt {
                        required: row.get(0)?,
                        own_check_key: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Record the own-check inputs a reader observed for an indexed user
    /// session, before it publishes anything read from them. The same key
    /// changes nothing, so a completed check stays completed. A different
    /// key moves the attempt on; a first key on a row that had none is only
    /// kept. `None` is a reader that could not observe them: a row that had
    /// a key moves on and keeps none, since nothing proves its inputs
    /// unchanged. A session without a row gets its first attempt. Returns
    /// the attempt the session requires afterwards, or `None` for a session
    /// that is not an indexed user session.
    pub fn observe_own_check(
        &mut self,
        session_id: &str,
        key: Option<&str>,
    ) -> Result<Option<Attempt>> {
        if let Some(key) = key {
            check_key(key)?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let user: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id=?1 AND kind='user')",
            [session_id],
            |row| row.get(0),
        )?;
        if !user {
            return Ok(None);
        }
        transaction.execute(
            "INSERT INTO session_child_checks(session_id,required_generation,own_check_key)
             VALUES (?1,1,?2)
             ON CONFLICT(session_id) DO UPDATE SET
               required_generation=required_generation
                 + (own_check_key IS NOT NULL AND own_check_key IS NOT excluded.own_check_key),
               own_check_key=excluded.own_check_key
             WHERE own_check_key IS NOT excluded.own_check_key",
            params![session_id, key],
        )?;
        let attempt = transaction.query_row(
            "SELECT required_generation,own_check_key FROM session_child_checks
             WHERE session_id=?1",
            [session_id],
            |row| {
                Ok(Attempt {
                    required: row.get(0)?,
                    own_check_key: row.get(1)?,
                })
            },
        )?;
        transaction.commit()?;
        Ok(Some(attempt))
    }

    /// Move the named sessions' attempts on: a check in flight for them no
    /// longer counts. Sessions without a row are passed over. Returns how
    /// many moved.
    pub fn invalidate_child_checks(&mut self, session_ids: &[&str]) -> Result<usize> {
        if session_ids.len() > MAX_INVALIDATIONS {
            return Err(crate::Error::InvalidInput(
                "too many sessions for one check invalidation",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut changed = 0;
        for session in session_ids {
            changed += transaction.execute(
                "UPDATE session_child_checks SET required_generation=required_generation+1
                 WHERE session_id=?1",
                [session],
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    /// Move on the attempt of every indexed user session of `host` with
    /// exactly this native identity: a launch naming it appeared or changed.
    pub fn invalidate_child_checks_of(&mut self, host: Host, native: &str) -> Result<usize> {
        invalidate_of(&self.connection, host, native)
    }

    /// A page of indexed user sessions, after `after` in identifier order,
    /// that are not known children and whose checks have not finished for
    /// the generation they require under today's detector versions. A
    /// session with no row yet (one never committed through an ingest batch,
    /// say) gets its first generation first.
    pub fn open_child_checks(
        &mut self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<OpenCheck>> {
        let limit = i64::try_from(limit.min(MAX_OPEN_CHECKS)).unwrap_or(0);
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO session_child_checks(session_id,required_generation)
             SELECT s.session_id,1 FROM sessions s
             WHERE s.kind='user' AND (?1 IS NULL OR s.session_id>?1)
               AND NOT EXISTS(SELECT 1 FROM session_child_checks k WHERE k.session_id=s.session_id)
             ORDER BY s.session_id LIMIT ?2",
            params![after, limit],
        )?;
        let rows = transaction
            .prepare(concat!(
                "SELECT s.session_id,s.host,s.native_session_id,k.required_generation,
                    s.started_at_ms,s.cwd,k.own_check_key
                 FROM sessions s JOIN session_child_checks k ON k.session_id=s.session_id
                 WHERE s.kind='user' AND (?1 IS NULL OR s.session_id>?1)
                   AND NOT ",
                checked_sql!(),
                " AND NOT ",
                known_child_sql!(),
                " ORDER BY s.session_id LIMIT ?2"
            ))?
            .query_map(params![after, limit], |row| {
                Ok(OpenCheck {
                    session_id: row.get(0)?,
                    host: row.get(1)?,
                    native_session_id: row.get(2)?,
                    required: row.get(3)?,
                    started_at_ms: row.get(4)?,
                    cwd: row.get(5)?,
                    own_check_key: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        transaction.commit()?;
        Ok(rows)
    }

    /// Mark the checks finished, under today's detector versions, for each
    /// session that still requires exactly the attempt and own-check key
    /// given. A session whose attempt or key moved on since, or that is
    /// already marked, is left as it is. Returns how many changed: each one a
    /// session a list now shows.
    pub fn settle_child_checks(&mut self, settled: &[(&str, i64, &str)]) -> Result<usize> {
        if settled.len() > MAX_OPEN_CHECKS {
            return Err(crate::Error::InvalidInput(
                "too many sessions for one check settlement",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut changed = 0;
        for (session, required, key) in settled {
            check_key(key)?;
            changed += transaction.execute(
                "UPDATE session_child_checks SET completed_generation=required_generation,
                     detector_versions=?3
                 WHERE session_id=?1 AND required_generation=?2 AND own_check_key=?4
                   AND (completed_generation IS NOT required_generation
                        OR detector_versions IS NOT ?3)",
                params![session, required, DETECTOR_VERSIONS, key],
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }
}

/// Move on the attempt of every indexed user session of `host` with exactly
/// this native identity, inside the caller's transaction.
pub(crate) fn invalidate_of(connection: &Connection, host: Host, native: &str) -> Result<usize> {
    Ok(connection.execute(
        "UPDATE session_child_checks SET required_generation=required_generation+1
         WHERE session_id IN (SELECT session_id FROM sessions
             WHERE host=?1 AND native_session_id=?2 AND kind='user')",
        params![host, native],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stored_versions_name_every_detector_a_check_runs() {
        assert_eq!(
            DETECTOR_VERSIONS,
            format!(
                "check1.spawn{}.scan{}.claude{}.cli{}.bash{}",
                crate::creation::CODEX_THREAD_SPAWN_VERSION,
                LAUNCH_VALIDATION_VERSION,
                crate::creation::CLAUDE_LAUNCH_CREATE_VERSION,
                crate::creation::CODEX_CLI_LAUNCH_VERSION,
                crate::child_fact::CLAUDE_BASH_CHILD_VERSION,
            )
        );
        assert!(DETECTOR_VERSIONS.len() <= 64);
    }
}
