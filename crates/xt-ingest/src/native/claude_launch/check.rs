//! Finishing each indexed session's display check
//! ([`xt_store::child_check`]): the step that lets a new conversation be
//! listed on its own once the supported checks of who created it have
//! finished without child evidence. It never says a person created it.
//!
//! A session's own inputs come first, each read the way its detector reads
//! them, and observed as its own-check key before anything else is decided:
//!
//! - Codex: the thread's own opening header, read by the typed-header pass
//!   ([`super::super::session_creation`]). A header that is this thread's
//!   own and records no spawn finishes the check, unless it is the fresh
//!   header a `codex exec` run writes ([`super::fresh_exec_opened`]): only
//!   such a thread can be a `codex exec --json` launch's child.
//! - Claude: the first eligible input of its one recorded transcript, read
//!   as a `Bash` launch's possible children are read. A history with no such
//!   input yet, or one that cannot be read, keeps checking.
//! - Every other host: no child detector exists, so nothing more is checked.
//!
//! A fresh `exec` thread and a Claude session then wait for the launches
//! that might have created them, as far as this version finds launches:
//!
//! - every Codex history the census finds has been read for launches since
//!   the check began and is still exactly what that read covered
//!   ([`super::LaunchBacklog::discovered_since`]); one that could not be
//!   checked keeps the session checking;
//! - no published launch naming exactly this session is still to be looked
//!   at, and no launch naming it, or (for a fresh `exec` thread) made before
//!   it opened, is still waiting for its first result
//!   ([`super::LaunchBacklog::unfinished_for`]). Another child's launches do
//!   not hold it;
//! - for a Claude session, the `Bash` checks ([`BashBacklog::clear_for`]).
//!
//! A check that began is bound to its attempt: a history it read that later
//! changes other than by growing begins every check in flight again. The
//! store writes a finished check only for the attempt and key captured
//! ([`Store::settle_child_checks`]). Each pass looks at one page of the
//! sessions still checking, after the passes of launch work it may wait on.
//!
//! Nothing is kept but identities, attempts, generations and digests.

use super::{
    Discovery, LaunchBacklog, LaunchLimits,
    bash::{self, BashBacklog, OwnFirst, OwnOpening, OwnRead},
    source::{Allowance, Budget},
};
use crate::native::readers_cli::CancelToken;
use std::{collections::HashMap, path::Path};
use xt_store::{Host, Store, child_check::OpenCheck, claude_launch::SegmentGeneration};

/// Sessions still checking one pass looks at.
pub const CHECK_PAGE: usize = 256;

/// The own-check key of a session no child detector reads: its host has none.
const NO_DETECTOR_KEY: &str = "u1:none";

/// Remembered own reads, beyond which the oldest are forgotten (and read
/// again if still needed).
const MAX_OWN: usize = 100_000;

/// Own reads of Claude openings kept in progress between passes at once;
/// another waits for one to end.
const MAX_READING: usize = 4;

/// A Codex thread's own header as the typed-header pass read it.
#[derive(Clone, Debug)]
struct CodexOwn {
    attempt: i64,
    key: String,
    fresh_exec: Option<i64>,
}

/// A Claude session's own opening as last read for its check.
#[derive(Clone, Debug)]
struct ClaudeOwn {
    attempt: i64,
    /// `None` when it could not be read, or not to its end.
    key: Option<String>,
    /// The transcript as the read saw it, if it saw one: a read that found
    /// no input, or could not end, is not made again until it changes.
    generation: Option<SegmentGeneration>,
    /// Its first eligible input, when it has one.
    first: Option<OwnFirst>,
}

/// Where a check that waits for launches began.
#[derive(Clone, Copy, Debug)]
struct Begun {
    attempt: i64,
    /// The census epoch then: launch reads must be newer.
    epoch: u64,
    /// The `Bash` pass then.
    seq: u64,
}

/// Display-check work kept between passes, in memory. A restarted worker
/// reads every session's own inputs again before it finishes a check; a
/// finished check whose own inputs read the same keeps standing.
#[derive(Debug, Default)]
pub struct CheckBacklog {
    codex: HashMap<String, CodexOwn>,
    claude: HashMap<String, ClaudeOwn>,
    begun: HashMap<String, Begun>,
    /// Codex threads whose header the typed-header pass is asked to read,
    /// and the attempt each was asked for, so a refused read is not asked
    /// for again until the attempt moves on or the thread is imported again.
    asked: HashMap<String, i64>,
    probes: Vec<String>,
    /// Claude opening reads in progress, by session, with the attempt each
    /// is for: resumed where the pass's budget stopped them.
    reading: HashMap<String, (i64, Box<OwnRead>)>,
    after: Option<String>,
    /// A page of sessions still checking is left for the next pass.
    more: bool,
    /// Something was imported since the last whole look.
    woken: bool,
    memory: Option<Allowance>,
}

impl CheckBacklog {
    /// The backlog a watcher starts with: every session still checking is
    /// looked at.
    pub fn starting() -> Self {
        Self {
            woken: true,
            ..Self::default()
        }
    }

    /// A scan of a host's sources ran — a change event, a rescan or the
    /// startup scan — whether or not its reader could import anything:
    /// every session still checking is looked at again, and own reads that
    /// were refused or found nothing are made again. A pass on a timer
    /// releases neither.
    pub fn wake(&mut self) {
        self.woken = true;
        self.asked.clear();
        self.claude.retain(|_, own| own.first.is_some());
    }

    /// Whether a later pass has anything to look at.
    pub fn pending(&self) -> bool {
        self.more || self.woken
    }

    /// A Codex thread's own header, as the typed-header pass read it for its
    /// indexed session at `attempt`. One a check asked for may have arrived
    /// after the check's page went past it: every session still checking is
    /// looked at once more. One no check asked for (an unchanged header read
    /// again) wakes nothing.
    pub fn note_codex(&mut self, session: &str, attempt: i64, fresh_exec: Option<i64>, key: &str) {
        if self.asked.remove(session).is_some() {
            self.woken = true;
        }
        if self.codex.len() >= MAX_OWN {
            self.codex.clear();
        }
        self.codex.insert(
            session.to_owned(),
            CodexOwn {
                attempt,
                key: key.to_owned(),
                fresh_exec,
            },
        );
    }

    /// The Codex threads whose header a check asked to be read.
    pub fn take_probes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.probes)
    }

    /// Whether `session` waits for launch discovery begun at its `attempt`,
    /// and since when; a check not begun begins now, with a new census.
    fn begun(
        &mut self,
        launches: &mut LaunchBacklog,
        bash: &BashBacklog,
        row: &OpenCheck,
    ) -> Option<Begun> {
        match self.begun.get(&row.session_id) {
            Some(begun) if begun.attempt == row.required => Some(*begun),
            _ => {
                self.begun.insert(
                    row.session_id.clone(),
                    Begun {
                        attempt: row.required,
                        epoch: launches.census_epoch(),
                        seq: bash.seq(),
                    },
                );
                launches.renew_census();
                None
            }
        }
    }
}

/// What one pass of display checks did. Counts only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckProgress {
    /// Sessions still checking that this pass looked at.
    pub looked: usize,
    /// Checks this pass finished: each a session a list now shows.
    pub settled: usize,
    /// Checks in flight begun again because a history they read changed.
    pub restarted: usize,
    /// Own reads that read further this pass, or ended.
    pub own_reads: usize,
    pub bytes_read: u64,
}

/// One bounded pass of display checks, within `limits`, after the launch
/// work of the same pass.
#[allow(clippy::too_many_arguments)]
pub fn continue_checks_into(
    store: &mut Store,
    home: &Path,
    checks: &mut CheckBacklog,
    launches: &mut LaunchBacklog,
    bash: &mut BashBacklog,
    limits: &LaunchLimits,
    cancel: Option<&CancelToken>,
    progress: &mut CheckProgress,
) -> xt_store::Result<()> {
    let mut budget = Budget::new(limits.max_pass_bytes, limits.deadline, cancel);
    let result = pass(
        store,
        home,
        checks,
        launches,
        bash,
        limits,
        &mut budget,
        progress,
    );
    progress.bytes_read += budget.spent;
    result
}

#[allow(clippy::too_many_arguments)]
fn pass(
    store: &mut Store,
    home: &Path,
    checks: &mut CheckBacklog,
    launches: &mut LaunchBacklog,
    bash: &mut BashBacklog,
    limits: &LaunchLimits,
    budget: &mut Budget<'_>,
    progress: &mut CheckProgress,
) -> xt_store::Result<()> {
    // A history a check in flight read changed other than by growing: each
    // such check begins again, at a new attempt.
    if launches.take_captured_changed() && !checks.begun.is_empty() {
        let ids: Vec<String> = checks.begun.drain().map(|(id, _)| id).collect();
        for chunk in ids.chunks(xt_store::child_check::MAX_INVALIDATIONS) {
            let chunk: Vec<&str> = chunk.iter().map(String::as_str).collect();
            progress.restarted += store.invalidate_child_checks(&chunk)?;
        }
    }
    let memory = checks
        .memory
        .get_or_insert_with(|| Allowance::new(limits.thread_memory))
        .clone();
    if checks.after.is_none() {
        checks.woken = false;
    }
    let page = store.open_child_checks(checks.after.as_deref(), CHECK_PAGE)?;
    checks.more = page.len() == CHECK_PAGE;
    checks.after = if checks.more {
        page.last().map(|row| row.session_id.clone())
    } else {
        None
    };
    let mut settled: Vec<(String, i64, String)> = Vec::new();
    for mut row in page {
        progress.looked += 1;
        match row.host {
            Host::Codex => {
                let Some(native) = row.native_session_id.clone() else {
                    continue;
                };
                let own = checks
                    .codex
                    .get(&row.session_id)
                    .filter(|own| {
                        own.attempt == row.required
                            && row.own_check_key.as_deref() == Some(own.key.as_str())
                    })
                    .cloned();
                let Some(own) = own else {
                    // Its header is to be read for this attempt, once.
                    if checks.asked.get(&row.session_id) != Some(&row.required) {
                        checks.asked.insert(row.session_id.clone(), row.required);
                        checks.probes.push(native);
                    }
                    continue;
                };
                let clear = match own.fresh_exec {
                    // An own header that no `codex exec --json` launch makes.
                    None => true,
                    Some(opened) => {
                        launched_clear(checks, launches, bash, store, &row, &native, Host::Codex)?
                            && !launches.unfinished_for(Host::Codex, &native, opened)
                    }
                };
                if clear {
                    settled.push((row.session_id.clone(), row.required, own.key));
                }
            }
            Host::Claude => {
                let Some(native) = row.native_session_id.clone() else {
                    continue;
                };
                let Some(first) = claude_own(
                    checks, store, home, &mut row, &native, limits, budget, &memory, progress,
                )?
                else {
                    continue;
                };
                if !launched_clear(checks, launches, bash, store, &row, &native, Host::Claude)?
                    || launches.unfinished_for(Host::Claude, &native, i64::MAX)
                {
                    continue;
                }
                let Some(begun) = checks.begun.get(&row.session_id).copied() else {
                    continue;
                };
                if bash.clear_for(store, home, &row.session_id, &native, &first, begun.seq)? {
                    settled.push((row.session_id.clone(), row.required, first.key(&native)));
                }
            }
            Host::Cursor | Host::Other => {
                let required = if row.own_check_key.as_deref() == Some(NO_DETECTOR_KEY) {
                    row.required
                } else {
                    match store.observe_own_check(&row.session_id, Some(NO_DETECTOR_KEY))? {
                        Some(attempt) => attempt.required,
                        None => continue,
                    }
                };
                settled.push((row.session_id.clone(), required, NO_DETECTOR_KEY.to_owned()));
            }
        }
    }
    if !settled.is_empty() {
        let rows: Vec<(&str, i64, &str)> = settled
            .iter()
            .map(|(id, required, key)| (id.as_str(), *required, key.as_str()))
            .collect();
        progress.settled += store.settle_child_checks(&rows)?;
        for (id, _, _) in &settled {
            checks.begun.remove(id);
            checks.codex.remove(id);
            checks.claude.remove(id);
            checks.asked.remove(id);
            checks.reading.remove(id);
        }
    }
    Ok(())
}

/// Whether the launches this version finds leave no doubt that no agent's
/// launch created this session: discovery since its check began is complete
/// and no published launch naming exactly it is still to be looked at.
fn launched_clear(
    checks: &mut CheckBacklog,
    launches: &mut LaunchBacklog,
    bash: &BashBacklog,
    store: &Store,
    row: &OpenCheck,
    native: &str,
    host: Host,
) -> xt_store::Result<bool> {
    let Some(begun) = checks.begun(launches, bash, row) else {
        return Ok(false);
    };
    if launches.discovered_since(begun.epoch) != Discovery::Complete {
        return Ok(false);
    }
    Ok(!store.claude_launch_child_open(native, host)?)
}

/// A Claude session's first eligible input for its current attempt, read
/// and observed as its own-check key first when it is not known for that
/// attempt. A read the pass's budget stopped is kept and resumed where it
/// stopped. A read that found no input, failed or could not end is not made
/// again until the transcript changes or a scan wakes the checks. `None`
/// keeps the session checking.
#[allow(clippy::too_many_arguments)]
fn claude_own(
    checks: &mut CheckBacklog,
    store: &mut Store,
    home: &Path,
    row: &mut OpenCheck,
    native: &str,
    limits: &LaunchLimits,
    budget: &mut Budget<'_>,
    memory: &Allowance,
    progress: &mut CheckProgress,
) -> xt_store::Result<Option<OwnFirst>> {
    if let Some(own) = checks.claude.get(&row.session_id)
        && own.attempt == row.required
    {
        if own.key.is_some()
            && own.key == row.own_check_key
            && let Some(first) = &own.first
        {
            return Ok(Some(first.clone()));
        }
        if own.first.is_none()
            && bash::transcript_generation(store, home, native)? == own.generation
        {
            return Ok(None);
        }
    }
    // A read in progress for another attempt is begun again.
    if checks
        .reading
        .get(&row.session_id)
        .is_some_and(|(attempt, _)| *attempt != row.required)
    {
        checks.reading.remove(&row.session_id);
    }
    if budget.exhausted() {
        checks.more = true;
        return Ok(None);
    }
    let read = match checks.reading.remove(&row.session_id) {
        Some((_, read)) => Ok(read),
        None if checks.reading.len() >= MAX_READING => {
            checks.more = true;
            return Ok(None);
        }
        None => OwnRead::start(store, home, native, limits.max_line, memory)?.map(Box::new),
    };
    let before = budget.spent;
    let found = match read {
        Ok(mut read) => match read.advance(budget) {
            OwnOpening::Paused => {
                if budget.spent > before {
                    progress.own_reads += 1;
                }
                checks.more = true;
                checks
                    .reading
                    .insert(row.session_id.clone(), (row.required, read));
                return Ok(None);
            }
            found => found,
        },
        Err(found) => found,
    };
    progress.own_reads += 1;
    let (observed, key, generation, first) = match found {
        OwnOpening::Input(generation, first) => {
            (true, Some(first.key(native)), Some(generation), Some(first))
        }
        OwnOpening::NoInput(generation) => (
            true,
            Some(bash::no_input_key(native)),
            Some(generation),
            None,
        ),
        // Nothing proves its inputs unchanged.
        OwnOpening::Failed(generation) => (true, None, generation, None),
        // Not read to its end: nothing observed.
        OwnOpening::Again(generation) => (false, None, generation, None),
        OwnOpening::Paused => unreachable!("kept above"),
    };
    let attempt = if observed {
        match store.observe_own_check(&row.session_id, key.as_deref())? {
            Some(attempt) => attempt,
            None => return Ok(None),
        }
    } else {
        xt_store::child_check::Attempt {
            required: row.required,
            own_check_key: row.own_check_key.clone(),
        }
    };
    row.required = attempt.required;
    row.own_check_key = attempt.own_check_key.clone();
    if checks.claude.len() >= MAX_OWN {
        checks.claude.clear();
    }
    checks.claude.insert(
        row.session_id.clone(),
        ClaudeOwn {
            attempt: attempt.required,
            key: key.filter(|_| observed),
            generation,
            first: first.clone(),
        },
    );
    Ok(first)
}

/// The own-check key of the Claude transcript at `path`, for session
/// `native`, as an importer about to read it from its first byte observes
/// it: `Some(None)` when it cannot be read, `None` when its read could not
/// end now (it changed while read), which observes nothing.
pub(in crate::native) fn claude_own_key_at(path: &Path, native: &str) -> Option<Option<String>> {
    let limits = LaunchLimits::default();
    let mut budget = Budget::new(limits.max_pass_bytes, limits.deadline, None);
    let memory = Allowance::new(limits.thread_memory);
    let found = match OwnRead::at(path, native, limits.max_line, &memory) {
        Ok(mut read) => read.advance(&mut budget),
        Err(found) => found,
    };
    match found {
        OwnOpening::Input(_, first) => Some(Some(first.key(native))),
        OwnOpening::NoInput(_) => Some(Some(bash::no_input_key(native))),
        OwnOpening::Failed(_) => Some(None),
        OwnOpening::Again(_) | OwnOpening::Paused => None,
    }
}
