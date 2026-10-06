//! Child facts for Claude sessions a Claude agent launched from its own
//! `Bash` tool, including sessions whose identifier Claude assigned itself and
//! nothing printed. Only that the session is an agent's child is recorded
//! ([`Store::record_child_facts`]); no parent relation is written here, so
//! another call that might also have launched the child, or an old caller
//! history that is gone, never stands in the way.
//!
//! A caller is an indexed Claude user session whose own recorded transcript
//! holds a supported `Bash` call ([`parent`], [`script`]): a finite literal
//! script of create-mode Claude print launches, each in a literal absolute
//! directory with a literal prompt, on the main chain of the transcript's own
//! verified session, closed by exactly one result of its own that is no
//! error, interrupted by nothing, wrote nothing to standard error and printed
//! each launch's complete answer at its place.
//!
//! The child of one launch is decided from the index and the saved
//! histories, never from a guess. The indexed Claude sessions first saying
//! anything between the call and its result locate the project folders a
//! child could be saved in. Every history in those folders is listed and its
//! opening read; the possible children are those whose first eligible input
//! is the launch's exact prompt, saved in the launch's directory or none,
//! dated within the call or undated. Exactly one must be possible — a folder
//! entry that cannot be read, or two possible children, decides nothing, and
//! two withhold any fact already resting on the launch. That one history
//! must then be, read whole, a fresh session of its own name (every line
//! naming it, none before the call), its first input the prompt, saved in the
//! launch's directory, unique as the first; its first turn's final answer
//! exactly the printed one, dated within the call; one indexed user session
//! whose one recorded transcript it is; and no other project may hold a file
//! of its name. A name the command or a JSON result gave must be that one.
//! Right before the write, the caller, every listed history and the child
//! are checked again: every file exactly as read.
//!
//! Work lives in memory in a [`BashBacklog`] and is done in bounded passes
//! that the watcher runs with what is left of its pass: a startup sweep over
//! every indexed caller, page by page; the callers that made a `Bash` call in
//! the hour before a newly imported session's first input; callers whose
//! launches had no result yet when read; and the launches waiting for their
//! child, looked at again when anything is imported. A caller is read again
//! only once its transcript changed. A restart sweeps again; every write is
//! an idempotent child fact. Only identifiers, positions and closed labels
//! are stored; prompts and answers are compared in memory, as digests.

mod parent;
mod script;

use super::LaunchLimits;
use super::command;
use super::resolve::{self, ChildStream, Recorded, Sources, Walk};
use super::source::{self, Allowance, Budget, Reserved, Unread};
use crate::native::readers_cli::CancelToken;
use crate::native::session_titles::{claude_contained, title_sources};
use parent::{First, Invocation, ParentScan, ScanError};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use xt_store::{
    Host, Store,
    child_fact::{CLAUDE_BASH_CHILD_VERSION, ChildEvidence, ChildFact, ChildFactDisposition},
    claude_launch::SegmentGeneration,
};

/// Callers one sweep page reads from the index.
const CALLER_PAGE: usize = 256;
/// How long before a newly imported session's first input a `Bash` call
/// that may have launched it was made.
const BIRTH_WINDOW_MS: i64 = 60 * 60 * 1000;
/// Callers one birth queues at most.
const MAX_BIRTH_CALLERS: usize = 256;
/// Indexed sessions born within one call that are looked at; more decide
/// nothing.
const MAX_BORN: usize = 1_000;
/// Project folders one launch's children may lie in.
const MAX_FOLDERS: usize = 16;
/// Openings remembered, by history file.
const MAX_OPENINGS: usize = 200_000;
/// Remembered queue, caller and birth entries.
const MAX_QUEUE: usize = 100_000;
/// Callers remembered as read whole with nothing left to decide.
const MAX_READ: usize = 100_000;
/// Launches waiting for their child.
const MAX_WAITING: usize = 4_096;
/// Accepted launches retained for later child imports. The shared memory
/// allowance also holds every invocation's prompt and answer.
const MAX_ACCEPTED: usize = MAX_READ;

/// A launch whose child could not be decided yet.
#[derive(Debug)]
struct Waiting {
    caller: String,
    path: PathBuf,
    generation: SegmentGeneration,
    invocation: Invocation,
    state: Option<CheckState>,
    _held: Reserved,
}

/// The part of a launch check that has already consumed a pass's limits.
#[derive(Debug)]
struct CheckState {
    folders: Vec<PathBuf>,
    folder_index: usize,
    listing: Option<std::fs::ReadDir>,
    opening: Option<ParentScan>,
    listed: Vec<Listed>,
    child_stream: Option<ChildStream>,
    projects_listing: Option<std::fs::ReadDir>,
    projects_stamp: Option<DirectoryStamp>,
    projects_done: bool,
    verify_folder: usize,
    verify_listing: Option<std::fs::ReadDir>,
    verify_names: Vec<PathBuf>,
    verify_stamps: Vec<DirectoryStamp>,
}

type DirectoryStamp = (u64, u64, i64, i64, i64, i64);

impl CheckState {
    fn new(folders: Vec<PathBuf>) -> Self {
        Self {
            folders,
            folder_index: 0,
            listing: None,
            opening: None,
            listed: Vec::new(),
            child_stream: None,
            projects_listing: None,
            projects_stamp: None,
            projects_done: false,
            verify_folder: 0,
            verify_listing: None,
            verify_names: Vec::new(),
            verify_stamps: Vec::new(),
        }
    }
}

/// One caller's transcript being read.
#[derive(Debug)]
struct Reading {
    native: String,
    scan: ParentScan,
}

/// Bash launch work not yet done. It lives with whoever scans, in memory: a
/// restarted worker sweeps every indexed caller again, and every write it
/// repeats changes nothing.
#[derive(Debug, Default)]
pub struct BashBacklog {
    /// Where the startup sweep is, while it runs.
    sweep: Option<Option<String>>,
    queue: VecDeque<String>,
    queued: HashSet<String>,
    births: VecDeque<String>,
    born: HashSet<String>,
    /// Callers whose read left launches without a result yet.
    open: HashSet<String>,
    read: HashMap<PathBuf, SegmentGeneration>,
    openings: HashMap<PathBuf, (SegmentGeneration, First)>,
    waiting: Vec<Waiting>,
    accepted: Vec<Waiting>,
    recheck: bool,
    recheck_accepted: bool,
    current: Option<Box<Reading>>,
    memory: Option<Allowance>,
    /// The number of the pass now running: a display check that began
    /// before a caller's read knows that read saw what it was waiting for.
    seq: u64,
    /// Every caller transcript read whole, as the generation and pass it
    /// was read at and whether this version could read it.
    scanned: HashMap<PathBuf, Scanned>,
    /// The supported launches each such read found, as metadata only.
    calls: HashMap<PathBuf, (String, Vec<CallSummary>)>,
    /// Caller transcripts whose last read ended unfinished, changed or out
    /// of memory, as they were then: not asked for again until they change.
    attempted: HashMap<PathBuf, SegmentGeneration>,
    #[cfg(test)]
    test_waiting_limit: Option<usize>,
    #[cfg(test)]
    test_accepted_limit: Option<usize>,
}

impl BashBacklog {
    /// The backlog a watcher starts with: a sweep over every indexed caller.
    pub fn starting() -> Self {
        Self {
            sweep: Some(None),
            ..Self::default()
        }
    }

    /// Claude sessions a scan imported: each may be a newly born child, or
    /// a caller whose launches now have their result.
    pub fn add_sessions<'a>(&mut self, natives: impl IntoIterator<Item = &'a str>) {
        for native in natives {
            self.recheck = true;
            self.recheck_accepted = true;
            if self.born.len() >= MAX_QUEUE {
                self.born.clear();
            }
            if self.born.insert(native.to_owned()) && self.births.len() < MAX_QUEUE {
                self.births.push_back(native.to_owned());
            }
            if self.open.remove(native) {
                self.enqueue(native.to_owned());
            }
        }
    }

    /// Keep what a caller's whole read found, for the display check.
    fn note_scanned(
        &mut self,
        path: &Path,
        native: &str,
        generation: SegmentGeneration,
        invocations: &[Invocation],
    ) {
        if self.scanned.len() >= MAX_READ {
            self.scanned.clear();
            self.calls.clear();
        }
        self.scanned.insert(
            path.to_path_buf(),
            Scanned {
                generation,
                _seq: self.seq,
                failed: false,
            },
        );
        let calls: Vec<CallSummary> = invocations
            .iter()
            .map(|invocation| CallSummary {
                call_id: invocation.call_id.clone(),
                index: invocation.index,
                launch_ms: invocation.launch_ms,
                end_ms: invocation.end_ms,
                answered: invocation.result.is_some(),
                cwd: invocation.cwd.clone(),
                prompt_digest: invocation.prompt_digest,
            })
            .collect();
        if calls.is_empty() {
            self.calls.remove(path);
        } else {
            self.calls
                .insert(path.to_path_buf(), (native.to_owned(), calls));
        }
    }

    /// The number of the pass now running, or last run.
    pub(super) fn seq(&self) -> u64 {
        self.seq
    }

    /// Whether the Claude `Bash` checks this version supports leave no doubt
    /// that the indexed Claude session `native`, whose first eligible input
    /// is `first`, was not launched from another Claude session's shell, as
    /// far as an in-flight display check needs: every Claude session written
    /// in the hour before that input has been read whole at its current
    /// generation and could be read; and no
    /// supported launch any read found — of any age — has this input's
    /// prompt digest and directory, was made before it and had not ended
    /// before it, unless a fact it rests on names another child. A caller
    /// still to read is queued. `false` keeps the session checking.
    pub(super) fn clear_for(
        &mut self,
        store: &Store,
        home: &Path,
        session_id: &str,
        native: &str,
        first: &OwnFirst,
        _since: u64,
    ) -> xt_store::Result<bool> {
        let Some(born) = first.ts_ms else {
            return Ok(false);
        };
        let callers = store.claude_sessions_active_between(
            born - BIRTH_WINDOW_MS,
            born,
            MAX_BIRTH_CALLERS + 1,
        )?;
        if callers.len() > MAX_BIRTH_CALLERS {
            return Ok(false);
        }
        let mut clear = true;
        for (_, caller) in callers {
            if caller == native {
                continue;
            }
            let Some(path) = transcript(store, home, &caller)? else {
                // A caller whose one transcript is not known cannot be read.
                return Ok(false);
            };
            let Ok(now) = source::stat(&path) else {
                clear = false;
                continue;
            };
            match self.scanned.get(&path) {
                Some(read) if read.generation == now => {
                    if read.failed {
                        return Ok(false);
                    }
                }
                // Its last read could not finish, and it has not changed.
                _ if self.attempted.get(&path) == Some(&now) => clear = false,
                _ => {
                    // Read it whole now, even if an earlier read decided it.
                    self.read.remove(&path);
                    self.enqueue(caller);
                    clear = false;
                }
            }
        }
        if !clear {
            return Ok(false);
        }
        let Some(digest) = first.digest else {
            // No one text: no launch's prompt can be this input.
            return Ok(true);
        };
        for (caller, calls) in self.calls.values() {
            for call in calls {
                let matches = call.prompt_digest == digest
                    && first.cwd.as_deref().is_none_or(|cwd| cwd == call.cwd)
                    && call.launch_ms <= born
                    && call.end_ms.is_none_or(|end| born <= end);
                if !matches {
                    continue;
                }
                if !call.answered {
                    return Ok(false);
                }
                let facts = store.child_launch_facts(
                    ChildEvidence::ClaudeBashLaunch,
                    caller,
                    None,
                    &call.call_id,
                    call.index,
                )?;
                if !facts
                    .iter()
                    .any(|(child, accepted)| *accepted && child != session_id)
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn enqueue(&mut self, native: String) {
        if self.queued.len() < MAX_QUEUE && self.queued.insert(native.clone()) {
            self.queue.push_back(native);
        }
    }

    fn waiting_limit(&self) -> usize {
        #[cfg(test)]
        if let Some(limit) = self.test_waiting_limit {
            return limit;
        }
        MAX_WAITING
    }

    fn accepted_limit(&self) -> usize {
        #[cfg(test)]
        if let Some(limit) = self.test_accepted_limit {
            return limit;
        }
        MAX_ACCEPTED
    }

    /// Whether a later pass has anything to do.
    pub fn pending(&self) -> bool {
        self.sweep.is_some()
            || !self.queue.is_empty()
            || !self.births.is_empty()
            || self.current.is_some()
            || (self.recheck
                && (!self.waiting.is_empty()
                    || (self.recheck_accepted && !self.accepted.is_empty())))
    }
}

/// One caller transcript read whole.
#[derive(Clone, Copy, Debug)]
struct Scanned {
    generation: SegmentGeneration,
    _seq: u64,
    /// Beyond this version's bounds, or not a file it reads: what it may
    /// have launched is not known.
    failed: bool,
}

/// One supported launch a caller's read found: when it was made and ended,
/// where, and the digest of its prompt. Never the prompt.
#[derive(Clone, Debug)]
struct CallSummary {
    call_id: String,
    index: u32,
    launch_ms: i64,
    end_ms: Option<i64>,
    /// The call has a readable result of its own.
    answered: bool,
    cwd: String,
    prompt_digest: [u8; 32],
}

/// What a display check needs of a Claude session's own opening: its first
/// eligible input's identity, time, directory and text digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OwnFirst {
    pub uuid: Option<String>,
    pub ts_ms: Option<i64>,
    pub cwd: Option<String>,
    pub digest: Option<[u8; 32]>,
}

impl OwnFirst {
    /// The own-check key of this first input: identifiers, time and digests.
    pub fn key(&self, native: &str) -> String {
        use sha2::Digest;
        let mut digest = sha2::Sha256::new();
        for part in [
            native.as_bytes(),
            self.uuid.as_deref().unwrap_or("").as_bytes(),
            self.ts_ms
                .map(|ms| ms.to_string())
                .unwrap_or_default()
                .as_bytes(),
            self.cwd.as_deref().unwrap_or("").as_bytes(),
        ] {
            digest.update(part);
            digest.update([0]);
        }
        digest.update(self.digest.unwrap_or_default());
        digest.update([u8::from(self.digest.is_some())]);
        format!("a1:{}", hex(&digest.finalize()))
    }
}

/// The generation of the indexed Claude session `native`'s one recorded
/// transcript now, if it has one.
pub(super) fn transcript_generation(
    store: &Store,
    home: &Path,
    native: &str,
) -> xt_store::Result<Option<SegmentGeneration>> {
    Ok(transcript(store, home, native)?.and_then(|path| source::stat(&path).ok()))
}

/// The own-check key of a Claude session whose complete history holds no
/// eligible input yet.
pub(super) fn no_input_key(native: &str) -> String {
    use sha2::Digest;
    let mut digest = sha2::Sha256::new();
    digest.update(native.as_bytes());
    digest.update(b"\0no input");
    format!("a1:{}", hex(&digest.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What reading a Claude session's own opening found.
#[derive(Debug)]
pub(super) enum OwnOpening {
    /// Its first eligible input, from the generation read.
    Input(SegmentGeneration, OwnFirst),
    /// The whole history read, as this generation, holds no eligible input.
    NoInput(SegmentGeneration),
    /// Not read: the index records no one transcript for it, or the
    /// transcript could not be read as one file of its own, or is beyond
    /// this version's bounds. Read again only once it changes.
    Failed(Option<SegmentGeneration>),
    /// Not read to its end now: the file changed while it was read, its last
    /// line is still being written, or its read does not fit now. Read
    /// again from its first byte, as it then is.
    Again(Option<SegmentGeneration>),
    /// The pass's budget ended first; the read resumes where it stopped.
    Paused,
}

/// One read of a Claude session's own opening in progress: the reviewed
/// transcript reader, kept with its open file, position and buffers between
/// passes, within the shared allowance.
#[derive(Debug)]
pub(super) struct OwnRead {
    scan: ParentScan,
}

impl OwnRead {
    /// Begin reading the opening of the indexed Claude session `native`'s
    /// one recorded transcript, the way a launch's possible children are
    /// read: through its first eligible input. `Err` says why it cannot
    /// begin.
    pub fn start(
        store: &Store,
        home: &Path,
        native: &str,
        max_line: usize,
        memory: &Allowance,
    ) -> xt_store::Result<Result<Self, OwnOpening>> {
        let Some(path) = transcript(store, home, native)? else {
            return Ok(Err(OwnOpening::Failed(None)));
        };
        Ok(Self::at(&path, native, max_line, memory))
    }

    /// [`Self::start`] of a transcript already located.
    // The refusal is made once per read, never kept: its size is no cost.
    #[allow(clippy::result_large_err)]
    pub fn at(
        path: &Path,
        native: &str,
        max_line: usize,
        memory: &Allowance,
    ) -> Result<Self, OwnOpening> {
        let now = source::stat(path).ok();
        match ParentScan::new(path, native, &[], true, max_line, memory) {
            Ok(scan) => Ok(Self { scan }),
            Err(ScanError::Changed | ScanError::Unfinished | ScanError::Memory) => {
                Err(OwnOpening::Again(now))
            }
            Err(ScanError::Refused) => Err(OwnOpening::Failed(now)),
        }
    }

    /// Read on within `budget`.
    pub fn advance(&mut self, budget: &mut Budget<'_>) -> OwnOpening {
        let generation = Some(self.scan.generation);
        match self.scan.advance(budget) {
            Ok(None) => OwnOpening::Paused,
            Ok(Some(facts)) => match facts.first {
                First::Input(input) => OwnOpening::Input(
                    facts.generation,
                    OwnFirst {
                        uuid: input.uuid,
                        ts_ms: input.ts_ms,
                        cwd: input.cwd,
                        digest: input.digest,
                    },
                ),
                First::NoInput => OwnOpening::NoInput(facts.generation),
                First::Unknown => OwnOpening::Failed(generation),
            },
            Err(ScanError::Changed | ScanError::Unfinished | ScanError::Memory) => {
                OwnOpening::Again(generation)
            }
            Err(ScanError::Refused) => OwnOpening::Failed(generation),
        }
    }
}

/// What one pass did. Counts only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BashProgress {
    /// Caller transcripts read whole.
    pub callers_read: usize,
    /// Caller transcripts that could not be read now, or are beyond bounds.
    pub callers_unreadable: usize,
    /// Supported launches whose child was looked for.
    pub launches: usize,
    /// Child facts recorded, or already stored.
    pub children: usize,
    pub already: usize,
    /// Launches with more than one possible child: undecided, and any fact
    /// resting on them withheld.
    pub ambiguous: usize,
    pub rejected: usize,
    /// Launches whose child is not indexed yet or whose sources cannot be
    /// read now.
    pub waiting: usize,
    /// Facts and withholdings that changed what is stored.
    pub changed: usize,
    pub bytes_read: u64,
    pub advanced: bool,
}

/// One bounded pass over the backlog, counting into `progress`.
pub fn continue_claude_bash_into(
    store: &mut Store,
    home: &Path,
    backlog: &mut BashBacklog,
    limits: &LaunchLimits,
    cancel: Option<&CancelToken>,
    recorded_at: i64,
    progress: &mut BashProgress,
) -> xt_store::Result<()> {
    let mut budget = Budget::new(limits.max_pass_bytes, limits.deadline, cancel)
        .with_entries(limits.max_pass_entries);
    let before = progress.clone();
    let result = pass(
        store,
        home,
        backlog,
        limits,
        &mut budget,
        recorded_at,
        progress,
    );
    progress.bytes_read += budget.spent;
    progress.advanced |= budget.spent > 0 || budget.entries > 0 || *progress != before;
    result
}

/// [`continue_claude_bash_into`] with its own progress.
pub fn continue_claude_bash(
    store: &mut Store,
    home: &Path,
    backlog: &mut BashBacklog,
    limits: &LaunchLimits,
    cancel: Option<&CancelToken>,
    recorded_at: i64,
) -> xt_store::Result<BashProgress> {
    let mut progress = BashProgress::default();
    continue_claude_bash_into(
        store,
        home,
        backlog,
        limits,
        cancel,
        recorded_at,
        &mut progress,
    )?;
    Ok(progress)
}

/// What one pass reads with.
struct Pass<'a> {
    home: &'a Path,
    programs: Vec<PathBuf>,
    limits: &'a LaunchLimits,
    memory: Allowance,
    recorded_at: i64,
}

fn pass(
    store: &mut Store,
    home: &Path,
    backlog: &mut BashBacklog,
    limits: &LaunchLimits,
    budget: &mut Budget<'_>,
    recorded_at: i64,
    progress: &mut BashProgress,
) -> xt_store::Result<()> {
    let memory = backlog
        .memory
        .get_or_insert_with(|| Allowance::new(limits.thread_memory))
        .clone();
    let run = Pass {
        home,
        programs: command::installed_programs(home),
        limits,
        memory,
        recorded_at,
    };
    backlog.seq += 1;
    while !budget.exhausted() && budget.entries < limits.max_pass_entries {
        // 1. A caller being read.
        if let Some(reading) = backlog.current.as_mut() {
            match reading.scan.advance(budget) {
                Ok(None) => return Ok(()),
                Ok(Some(facts)) => {
                    let reading = backlog.current.take().expect("reading");
                    progress.callers_read += 1;
                    let path = reading.scan.path.clone();
                    let invocations = facts.invocations.unwrap_or_default();
                    backlog.note_scanned(&path, &reading.native, facts.generation, &invocations);
                    if invocations
                        .iter()
                        .any(|invocation| invocation.result_record.is_none())
                    {
                        backlog.open.insert(reading.native.clone());
                    }
                    let mut settled = true;
                    for invocation in invocations {
                        if invocation.answer.is_none() || invocation.result.is_none() {
                            continue;
                        }
                        progress.launches += 1;
                        let waiting = Waiting {
                            caller: reading.native.clone(),
                            path: path.clone(),
                            generation: facts.generation,
                            state: None,
                            _held: match run.memory.reserve(invocation.bytes()) {
                                Ok(held) => held,
                                Err(_) => {
                                    settled = false;
                                    continue;
                                }
                            },
                            invocation,
                        };
                        if !matches!(
                            decide_into(store, &run, backlog, budget, waiting, progress)?,
                            Intake::Kept
                        ) {
                            settled = false;
                        }
                    }
                    if settled && backlog.read.len() < MAX_READ {
                        backlog.read.insert(path, facts.generation);
                    }
                }
                Err(ScanError::Changed | ScanError::Unfinished | ScanError::Memory) => {
                    // Read again once it is imported again.
                    let reading = backlog.current.take().expect("reading");
                    if backlog.attempted.len() >= MAX_READ {
                        backlog.attempted.clear();
                    }
                    backlog
                        .attempted
                        .insert(reading.scan.path.clone(), reading.scan.generation);
                    backlog.open.insert(reading.native);
                    progress.callers_unreadable += 1;
                }
                Err(ScanError::Refused) => {
                    let reading = backlog.current.take().expect("reading");
                    if backlog.scanned.len() >= MAX_READ {
                        backlog.scanned.clear();
                    }
                    backlog.scanned.insert(
                        reading.scan.path.clone(),
                        Scanned {
                            generation: reading.scan.generation,
                            _seq: backlog.seq,
                            failed: true,
                        },
                    );
                    backlog.calls.remove(&reading.scan.path);
                    if backlog.read.len() < MAX_READ {
                        backlog
                            .read
                            .insert(reading.scan.path.clone(), reading.scan.generation);
                    }
                    progress.callers_unreadable += 1;
                }
            }
            continue;
        }
        // 2. A newly imported session: the callers that may have launched it.
        if let Some(native) = backlog.births.pop_front() {
            if let Some(born) = store.claude_first_input_ms(&native)? {
                for (_, caller) in store.claude_bash_callers_between(
                    born - BIRTH_WINDOW_MS,
                    born,
                    MAX_BIRTH_CALLERS,
                )? {
                    if caller != native {
                        backlog.enqueue(caller);
                    }
                }
            }
            continue;
        }
        // 3. Launches waiting for their child, once something was imported.
        if backlog.recheck
            && (!backlog.waiting.is_empty()
                || (backlog.recheck_accepted && !backlog.accepted.is_empty()))
        {
            backlog.recheck = false;
            let mut deferred = false;
            let waiting = std::mem::take(&mut backlog.waiting);
            let mut rest = waiting.into_iter();
            for item in rest.by_ref() {
                if pass_limit(budget, limits) {
                    backlog.waiting.push(item);
                    backlog.recheck = true;
                    deferred = true;
                    break;
                }
                if let Intake::Deferred(item) =
                    decide_into(store, &run, backlog, budget, item, progress)?
                {
                    backlog.waiting.push(*item);
                    backlog.recheck = true;
                    deferred = true;
                    break;
                }
            }
            backlog.waiting.extend(rest);
            if backlog.recheck_accepted {
                backlog.recheck_accepted = false;
                let accepted = std::mem::take(&mut backlog.accepted);
                let mut rest = accepted.into_iter();
                for mut item in rest.by_ref() {
                    let no_slot = backlog.waiting.len() >= backlog.waiting_limit();
                    if pass_limit(budget, limits) || no_slot {
                        backlog.accepted.push(item);
                        backlog.recheck_accepted = true;
                        // A full list of unchanged Wait items cannot make
                        // progress without another import. An interrupted
                        // pass or a Paused/Retry item can continue now.
                        backlog.recheck |= !no_slot;
                        deferred = true;
                        break;
                    }
                    // A new import can add a history to a folder. Reuse the
                    // caller's launch and re-list the possible children.
                    item.state = None;
                    if let Intake::Deferred(item) =
                        decide_into(store, &run, backlog, budget, item, progress)?
                    {
                        backlog.accepted.push(*item);
                        backlog.recheck_accepted = true;
                        // decide_into sets recheck for Paused/Retry. A full
                        // list of unchanged Wait items sleeps until import.
                        deferred = true;
                        break;
                    }
                }
                backlog.accepted.extend(rest);
            }
            if deferred {
                return Ok(());
            }
            continue;
        }
        backlog.recheck = false;
        // 4. The next caller.
        if let Some(native) = backlog.queue.pop_front() {
            backlog.queued.remove(&native);
            if let Some(path) = transcript(store, home, &native)? {
                let unchanged =
                    source::stat(&path).is_ok_and(|now| backlog.read.get(&path) == Some(&now));
                if !unchanged {
                    match ParentScan::new(
                        &path,
                        &native,
                        &run.programs,
                        false,
                        limits.max_line,
                        &run.memory,
                    ) {
                        Ok(scan) => {
                            backlog.current = Some(Box::new(Reading { native, scan }));
                        }
                        Err(_) => progress.callers_unreadable += 1,
                    }
                }
            }
            continue;
        }
        // 5. The startup sweep, a page at a time.
        if let Some(after) = backlog.sweep.as_mut() {
            let page = store.claude_bash_callers(after.as_deref(), CALLER_PAGE)?;
            match page.last() {
                Some((last, _)) => *after = Some(last.clone()),
                None => backlog.sweep = None,
            }
            for (_, native) in page {
                backlog.enqueue(native);
            }
            continue;
        }
        break;
    }
    Ok(())
}

/// Decide one launch and keep it waiting if it cannot be decided now.
/// Returns whether it is decided or kept: `false` only when it had to be
/// dropped, so that its caller is read again.
fn decide_into(
    store: &mut Store,
    run: &Pass<'_>,
    backlog: &mut BashBacklog,
    budget: &mut Budget<'_>,
    mut waiting: Waiting,
    progress: &mut BashProgress,
) -> xt_store::Result<Intake> {
    let decision = decide(store, run, backlog, budget, &mut waiting)?;
    match decision {
        Decision::Recorded(changed) => {
            progress.children += 1;
            progress.changed += changed;
            return Ok(retain_accepted(backlog, waiting));
        }
        Decision::Already => {
            progress.already += 1;
            if store
                .child_launch_facts(
                    ChildEvidence::ClaudeBashLaunch,
                    &waiting.caller,
                    None,
                    &waiting.invocation.call_id,
                    waiting.invocation.index,
                )?
                .iter()
                .any(|(_, accepted)| *accepted)
            {
                return Ok(retain_accepted(backlog, waiting));
            }
        }
        Decision::Ambiguous(changed) => {
            progress.ambiguous += 1;
            progress.changed += changed;
        }
        Decision::Rejected => progress.rejected += 1,
        Decision::Wait | Decision::Paused | Decision::Retry => {
            progress.waiting += 1;
            if matches!(decision, Decision::Paused | Decision::Retry) {
                backlog.recheck = true;
            }
            if decision != Decision::Paused {
                waiting.state = None;
            }
            if backlog.waiting.len() >= backlog.waiting_limit() {
                return Ok(Intake::Deferred(Box::new(waiting)));
            }
            backlog.waiting.push(waiting);
        }
    }
    Ok(Intake::Kept)
}

enum Intake {
    Kept,
    /// This invocation needs the waiting list, which is full. Keep it in
    /// the accepted queue until a later bounded pass can consider it.
    Deferred(Box<Waiting>),
    /// A valid accepted fact has no cache slot. Only its relevant caller
    /// must be read again on the next matching child import.
    Uncached,
}

fn retain_accepted(backlog: &mut BashBacklog, mut waiting: Waiting) -> Intake {
    // Completed streams, directory iterators and history listings are never
    // retained for the next import. Only the launch evidence is needed.
    waiting.state = None;
    if backlog.accepted.len() < backlog.accepted_limit() {
        backlog.accepted.push(waiting);
        Intake::Kept
    } else {
        // A valid stored fact stays accepted. An uncached launch is revisited
        // by rereading only this caller on a later relevant child import.
        backlog.read.remove(&waiting.path);
        Intake::Uncached
    }
}

fn pass_limit(budget: &Budget<'_>, limits: &LaunchLimits) -> bool {
    budget.exhausted() || budget.entries >= limits.max_pass_entries
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decision {
    Recorded(usize),
    /// The launch already has a stored fact.
    Already,
    Ambiguous(usize),
    Rejected,
    /// Its child is not indexed yet, or a source cannot be read now.
    Wait,
    /// The pass's budget ended within it.
    Paused,
    /// A paused directory changed; select all possible children again.
    Retry,
}

/// The one primary transcript the index recorded for a Claude session.
fn transcript(store: &Store, home: &Path, native: &str) -> xt_store::Result<Option<PathBuf>> {
    let projects = home.join(".claude/projects");
    let mut roots = vec![projects.clone()];
    if let Ok(resolved) = projects.canonicalize()
        && resolved != projects
    {
        roots.push(resolved);
    }
    let paths: Vec<PathBuf> = title_sources(store, Host::Claude, native)?
        .into_iter()
        .filter_map(|source| source.locator.strip_prefix("claude:").map(PathBuf::from))
        .collect();
    Ok(match paths.as_slice() {
        [path] if claude_contained(path, &roots, native) => Some(path.clone()),
        _ => None,
    })
}

/// Check only that an indexed Claude session still has exactly one readable
/// own transcript. Startup uses this to withdraw a saved completion when its
/// source vanished, without rereading an unchanged or appended body.
pub(in crate::native) fn transcript_readable(
    store: &Store,
    home: &Path,
    native: &str,
) -> xt_store::Result<bool> {
    let Some(path) = transcript(store, home, native)? else {
        return Ok(false);
    };
    let Ok((mut file, before)) = source::open(&path) else {
        return Ok(false);
    };
    let mut first = [0_u8; 1];
    Ok(file.read(&mut first).is_ok() && source_identity_still_at_path(&file, &path, before))
}

/// A growing transcript remains the same readable source. The full
/// generation belongs to the scanner's input checks, not this presence check.
fn source_identity_still_at_path(
    file: &std::fs::File,
    path: &Path,
    opened: SegmentGeneration,
) -> bool {
    let same_file = |current: SegmentGeneration| {
        current.device == opened.device && current.inode == opened.inode
    };
    source::generation_of(file).ok().is_some_and(same_file)
        && source::stat(path).ok().is_some_and(same_file)
}

/// One listed history of a project folder and what its opening says.
#[derive(Debug)]
struct Listed {
    path: PathBuf,
    generation: SegmentGeneration,
    first: First,
}

/// Continue the folder listings and the one opening in progress. A pass may
/// stop at any byte or entry without starting that work over.
fn list(
    run: &Pass<'_>,
    backlog: &mut BashBacklog,
    budget: &mut Budget<'_>,
    state: &mut CheckState,
) -> Option<Result<(), ()>> {
    while state.folder_index < state.folders.len() {
        if let Some(scan) = state.opening.as_mut() {
            match scan.advance(budget) {
                Ok(None) => return None,
                Ok(Some(facts)) => {
                    let scan = state.opening.take().expect("opening");
                    if backlog.openings.len() >= MAX_OPENINGS {
                        backlog.openings.clear();
                    }
                    backlog
                        .openings
                        .insert(scan.path.clone(), (facts.generation, facts.first.clone()));
                    state.listed.push(Listed {
                        path: scan.path,
                        generation: facts.generation,
                        first: facts.first,
                    });
                }
                Err(_) => {
                    let scan = state.opening.take().expect("opening");
                    state.listed.push(Listed {
                        path: scan.path,
                        generation: scan.generation,
                        first: First::Unknown,
                    });
                }
            }
            continue;
        }
        if state.listing.is_none() {
            let Ok(entries) = std::fs::read_dir(&state.folders[state.folder_index]) else {
                return Some(Err(()));
            };
            state.listing = Some(entries);
        }
        if !budget.entry() {
            return None;
        }
        let entry = match state.listing.as_mut().expect("listing").next() {
            None => {
                state.listing = None;
                state.folder_index += 1;
                continue;
            }
            Some(Ok(entry)) => entry,
            Some(Err(_)) => return Some(Err(())),
        };
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(now) = source::stat(&path) else {
            // Not a regular file of its own (an alias or worse) cannot be
            // ruled out as a child.
            return Some(Err(()));
        };
        // Cached opening bytes belong only to this exact generation.
        if let Some((then, first)) = backlog.openings.get(&path)
            && now == *then
            && !matches!(first, First::Unknown)
        {
            state.listed.push(Listed {
                path,
                generation: now,
                first: first.clone(),
            });
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_owned();
        match ParentScan::new(
            &path,
            &name,
            &run.programs,
            true,
            run.limits.max_line,
            &run.memory,
        ) {
            Ok(scan) => state.opening = Some(scan),
            Err(_) => {
                state.listed.push(Listed {
                    path,
                    generation: now,
                    first: First::Unknown,
                });
            }
        }
    }
    state.listed.sort_by(|a, b| a.path.cmp(&b.path));
    Some(Ok(()))
}

/// Whether `ts` lies within the call: from its record to its result.
fn within(invocation: &Invocation, ts: i64) -> bool {
    invocation.launch_ms <= ts && invocation.end_ms.is_none_or(|end| ts <= end)
}

fn decide(
    store: &mut Store,
    run: &Pass<'_>,
    backlog: &mut BashBacklog,
    budget: &mut Budget<'_>,
    waiting: &mut Waiting,
) -> xt_store::Result<Decision> {
    let invocation = &waiting.invocation;
    let kind = ChildEvidence::ClaudeBashLaunch;
    let call = invocation.call_id.as_str();
    let index = invocation.index;
    // A launch whose facts are all withheld stays so. One with an accepted
    // fact is only checked to still have exactly that one possible child.
    let stored = store.child_launch_facts(kind, &waiting.caller, None, call, index)?;
    if !stored.is_empty() && stored.iter().all(|(_, accepted)| !accepted) {
        return Ok(Decision::Already);
    }
    let (Some(answer), Some((_, result_ms))) = (invocation.answer, invocation.result.clone())
    else {
        return Ok(Decision::Rejected);
    };
    // A changed caller must be read again before any cached launch can decide.
    if !caller_current(backlog, waiting) {
        return Ok(Decision::Rejected);
    }
    // The project folders the indexed sessions born within the call in the
    // launch's directory (or none recorded) lie in. Every history saved for
    // one directory lies in one folder, so a possible child the index does
    // not hold yet is still listed.
    let born = store.claude_sessions_born_between(invocation.launch_ms, result_ms, MAX_BORN + 1)?;
    if born.len() > MAX_BORN {
        return Ok(Decision::Wait);
    }
    let mut folders: Vec<PathBuf> = Vec::new();
    for (native, cwd) in &born {
        if *native == waiting.caller || cwd.as_deref().is_some_and(|cwd| cwd != invocation.cwd) {
            continue;
        }
        let Some(path) = transcript(store, run.home, native)? else {
            continue;
        };
        if let Some(folder) = path.parent()
            && !folders.iter().any(|known| known == folder)
        {
            folders.push(folder.to_path_buf());
        }
    }
    // A stored fact whose child is no longer found stays: a deleted history
    // is no contradiction.
    if folders.is_empty() {
        return Ok(if stored.is_empty() {
            Decision::Wait
        } else {
            Decision::Already
        });
    }
    if folders.len() > MAX_FOLDERS {
        return Ok(Decision::Wait);
    }
    if waiting
        .state
        .as_ref()
        .is_some_and(|state| state.folders != folders)
    {
        waiting.state = None;
    }
    let state = waiting
        .state
        .get_or_insert_with(|| CheckState::new(folders.clone()));
    match list(run, backlog, budget, state) {
        None => return Ok(Decision::Paused),
        Some(Err(())) => return Ok(Decision::Wait),
        Some(Ok(())) => {}
    }
    let mut possible = Vec::new();
    for entry in &state.listed {
        match &entry.first {
            First::Unknown => return Ok(Decision::Wait),
            First::NoInput => {}
            First::Input(input) => {
                if input.digest == Some(invocation.prompt_digest)
                    && input.cwd.as_deref().is_none_or(|cwd| cwd == invocation.cwd)
                    && input.ts_ms.is_none_or(|ts| within(invocation, ts))
                {
                    possible.push(entry);
                }
            }
        }
    }
    let child = match possible.as_slice() {
        [] if !stored.is_empty() => return Ok(Decision::Already),
        [] => return Ok(Decision::Wait),
        [one] => *one,
        _ => {
            let changed = store.withhold_child_launch(kind, &waiting.caller, None, call, index)?;
            return Ok(Decision::Ambiguous(changed));
        }
    };
    if !stored.is_empty() {
        let name = child.path.file_stem().and_then(|stem| stem.to_str());
        let same = match name {
            Some(name) => {
                let holders = store.user_sessions_with_native(Host::Claude, name)?;
                stored
                    .iter()
                    .all(|(stored, _)| holders.as_slice() == std::slice::from_ref(stored))
            }
            None => false,
        };
        if same {
            return Ok(Decision::Already);
        }
        // Another history is now the one possible child: what the launch
        // created is no longer known.
        let changed = store.withhold_child_launch(kind, &waiting.caller, None, call, index)?;
        return Ok(Decision::Ambiguous(changed));
    }
    let First::Input(input) = &child.first else {
        unreachable!("possible children have an input");
    };
    let (Some(born_ms), Some(first_uuid)) = (input.ts_ms, input.uuid.clone()) else {
        return Ok(Decision::Rejected);
    };
    if input.cwd.as_deref() != Some(invocation.cwd.as_str()) {
        return Ok(Decision::Rejected);
    }
    let Some(native) = child
        .path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| command::uuid(stem))
        .map(str::to_owned)
    else {
        return Ok(Decision::Rejected);
    };
    if invocation
        .named
        .as_ref()
        .is_some_and(|named| *named != native)
    {
        return Ok(Decision::Rejected);
    }
    // The child as the index holds it: one user session whose one recorded
    // transcript is this very file.
    let child_session = match store
        .user_sessions_with_native(Host::Claude, &native)?
        .as_slice()
    {
        [] => return Ok(Decision::Wait),
        [one] => one.clone(),
        _ => return Ok(Decision::Rejected),
    };
    match transcript(store, run.home, &native)? {
        Some(path) if same_file(&path, &child.path) => {}
        Some(_) => return Ok(Decision::Rejected),
        None => return Ok(Decision::Wait),
    }
    // The child read whole: fresh, its own, its first input the prompt and
    // its first turn's answer the printed one.
    let unfinished = HashMap::new();
    let sources = Sources {
        home: run.home,
        programs: &run.programs,
        max_line: run.limits.max_line,
        max_child_bytes: run.limits.max_child_bytes,
        max_child_lines: run.limits.max_child_lines,
        max_window_bytes: run.limits.max_window_bytes,
        unfinished: &unfinished,
    };
    if state.child_stream.is_none() {
        let Ok(recorded) = Recorded::new(&invocation.prompt, invocation.launch_ms, &run.memory)
        else {
            return Ok(Decision::Wait);
        };
        let mut stream =
            match ChildStream::open(&child.path, &native, recorded, &sources, &run.memory) {
                Ok(stream) => stream,
                Err(Unread::TooLarge | Unread::Alias) => return Ok(Decision::Rejected),
                Err(_) => return Ok(Decision::Wait),
            };
        stream.turn = Some(Box::default());
        state.child_stream = Some(stream);
    }
    let stream = state.child_stream.as_mut().expect("child stream");
    let first = match stream.advance(budget) {
        Ok(None) => return Ok(Decision::Paused),
        Ok(Some(Ok(first))) => first,
        Ok(Some(Err(_))) => return Ok(Decision::Rejected),
        Err(Unread::TooLarge | Unread::Alias) => return Ok(Decision::Rejected),
        Err(_) => return Ok(Decision::Wait),
    };
    let turn = stream.turn.as_ref().cloned().unwrap_or_default();
    let Some(said) = turn.answer() else {
        return Ok(Decision::Rejected);
    };
    if first != first_uuid
        || turn.cwd.as_deref() != Some(invocation.cwd.as_str())
        || turn.first_ms != Some(born_ms)
        || said.digest != answer
        || said.ts_ms < born_ms
        || said.ts_ms > result_ms
        || stream.generation != child.generation
    {
        return Ok(Decision::Rejected);
    }
    // No other project holds a file of its name.
    let Some(projects) = child
        .path
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
    else {
        return Ok(Decision::Rejected);
    };
    let child_generation = stream.generation;
    if !state.projects_done {
        if state.projects_listing.is_none() {
            state.projects_stamp = directory_stamp(&projects);
            if state.projects_stamp.is_none() {
                return Ok(Decision::Wait);
            }
            let Ok(listing) = std::fs::read_dir(&projects) else {
                return Ok(Decision::Wait);
            };
            state.projects_listing = Some(listing);
        }
        let name = format!("{native}.jsonl");
        match resolve::sole_transcript(
            &child.path,
            &name,
            state.projects_listing.as_mut().expect("projects"),
            budget,
        ) {
            Walk::Paused => return Ok(Decision::Paused),
            Walk::Other => return Ok(Decision::Rejected),
            Walk::Unreadable => return Ok(Decision::Wait),
            Walk::Sole => state.projects_done = true,
        }
    }
    // A project folder added while the bounded walk was paused could be
    // missed by its open iterator. Do not use that walk after a change.
    match project_walk_current(state, &projects) {
        Verify::Changed => return Ok(Decision::Retry),
        Verify::Unreadable => return Ok(Decision::Wait),
        Verify::Complete => {}
        Verify::Paused => unreachable!("a directory stamp does not pause"),
    }
    // Right before the write: every source is exactly as read, and nothing added.
    if source::stat(&child.path).ok() != Some(child_generation)
        || source::stat(&waiting.path).ok() != Some(waiting.generation)
    {
        return Ok(Decision::Wait);
    }
    match verify_folders(state, budget) {
        Verify::Paused => return Ok(Decision::Paused),
        Verify::Changed => return Ok(Decision::Retry),
        Verify::Unreadable => return Ok(Decision::Wait),
        Verify::Complete => {}
    }
    // The final folder listings can span passes. A new project folder added
    // during them invalidates the earlier uniqueness walk too.
    match project_walk_current(state, &projects) {
        Verify::Changed => return Ok(Decision::Retry),
        Verify::Unreadable => return Ok(Decision::Wait),
        Verify::Complete => {}
        Verify::Paused => unreachable!("a directory stamp does not pause"),
    }
    for entry in &state.listed {
        if source::stat(&entry.path).ok() != Some(entry.generation) {
            return Ok(Decision::Wait);
        }
    }
    let fact = ChildFact {
        child_session_id: child_session,
        child_host: Host::Claude,
        child_native_session_id: native,
        evidence_kind: kind,
        evidence_version: CLAUDE_BASH_CHILD_VERSION,
        source_native_session_id: waiting.caller.clone(),
        source_rollout_id: None,
        launch_call_id: Some(invocation.call_id.clone()),
        launch_operation_index: Some(invocation.index),
        first_record_uuid: Some(first_uuid),
    };
    // Verification may have paused after the earlier caller check. The
    // caller need not be in a candidate child's project folder.
    if !caller_current(backlog, waiting) {
        return Ok(Decision::Rejected);
    }
    let report = store.record_child_facts(&[fact], run.recorded_at)?;
    Ok(match report.dispositions.first() {
        Some(ChildFactDisposition::Recorded | ChildFactDisposition::AlreadyRecorded) => {
            Decision::Recorded(report.changed)
        }
        Some(ChildFactDisposition::Withheld) => Decision::Ambiguous(report.changed),
        // The index may not hold yet what the transcript already does.
        Some(ChildFactDisposition::Abstained(_)) | None => Decision::Wait,
    })
}

/// Whether two paths name the same file.
fn same_file(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

fn caller_current(backlog: &mut BashBacklog, waiting: &Waiting) -> bool {
    if source::stat(&waiting.path).ok() == Some(waiting.generation) {
        return true;
    }
    backlog.enqueue(waiting.caller.clone());
    backlog.read.remove(&waiting.path);
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verify {
    Paused,
    Changed,
    Unreadable,
    Complete,
}

/// Finish the fresh listing of every candidate folder. A retained iterator
/// is usable only while that same folder's identity and change times hold.
fn verify_folders(state: &mut CheckState, budget: &mut Budget<'_>) -> Verify {
    while state.verify_folder < state.folders.len() {
        let folder = &state.folders[state.verify_folder];
        if state.verify_listing.is_none() {
            let Some(stamp) = directory_stamp(folder) else {
                return Verify::Unreadable;
            };
            let Ok(entries) = std::fs::read_dir(folder) else {
                return Verify::Unreadable;
            };
            state.verify_stamps.push(stamp);
            state.verify_listing = Some(entries);
        }
        let Some(now) = directory_stamp(folder) else {
            return Verify::Unreadable;
        };
        if Some(now) != state.verify_stamps.get(state.verify_folder).copied() {
            return Verify::Changed;
        }
        loop {
            if !budget.entry() {
                return Verify::Paused;
            }
            let entry = match state
                .verify_listing
                .as_mut()
                .expect("verification listing")
                .next()
            {
                None => break,
                Some(Ok(entry)) => entry,
                Some(Err(_)) => return Verify::Unreadable,
            };
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                state.verify_names.push(path);
            }
        }
        let Some(now) = directory_stamp(folder) else {
            return Verify::Unreadable;
        };
        if Some(now) != state.verify_stamps.get(state.verify_folder).copied() {
            return Verify::Changed;
        }
        state.verify_names.sort();
        let before: Vec<&PathBuf> = state
            .listed
            .iter()
            .filter(|entry| entry.path.parent() == Some(folder.as_path()))
            .map(|entry| &entry.path)
            .collect();
        if state.verify_names.iter().collect::<Vec<_>>() != before {
            return Verify::Changed;
        }
        state.verify_names.clear();
        state.verify_listing = None;
        state.verify_folder += 1;
    }
    for (folder, stamp) in state.folders.iter().zip(&state.verify_stamps) {
        match directory_stamp(folder) {
            Some(now) if now == *stamp => {}
            Some(_) => return Verify::Changed,
            None => return Verify::Unreadable,
        }
    }
    Verify::Complete
}

fn directory_stamp(path: &Path) -> Option<DirectoryStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((
        meta.dev(),
        meta.ino(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec(),
    ))
}

fn project_walk_current(state: &CheckState, projects: &Path) -> Verify {
    match (state.projects_stamp, directory_stamp(projects)) {
        (Some(before), Some(now)) if before == now => Verify::Complete,
        (Some(_), Some(_)) => Verify::Changed,
        _ => Verify::Unreadable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::{Read, Write},
        time::Duration,
    };
    use xt_store::{CanonicalRecord, SessionMeta, SessionSource, batch::SourceCursor};

    #[test]
    fn append_between_open_and_identity_checks_keeps_the_source_readable() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("source.jsonl");
        fs::write(&path, b"first\n").unwrap();
        let (mut opened, before) = source::open(&path).unwrap();
        let mut first = [0_u8; 1];
        opened.read_exact(&mut first).unwrap();
        assert_eq!(first, [b'f']);
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"later\n")
            .unwrap();
        assert_ne!(source::stat(&path).unwrap(), before);
        assert!(source_identity_still_at_path(&opened, &path, before));

        let old = temp.path().join("old.jsonl");
        fs::rename(&path, &old).unwrap();
        fs::write(&path, b"replacement\n").unwrap();
        assert!(!source_identity_still_at_path(&opened, &path, before));
        fs::remove_file(&path).unwrap();
        assert!(!source_identity_still_at_path(&opened, &path, before));
    }

    #[test]
    fn a_caller_append_after_a_read_prevents_stale_negative_completion() {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path();
        let native = "0c000000-0000-4000-8000-0000000000c1";
        let folder = home.join(".claude/projects/project");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join(format!("{native}.jsonl"));
        fs::write(&path, b"first\n").unwrap();
        let mut store = Store::open(home.join("index.sqlite")).unwrap();
        let mut meta = SessionMeta::new(native, "claude", SessionSource::Transcript);
        meta.native_session_id = Some(native.into());
        store.upsert_session(&meta, false).unwrap();
        let input: CanonicalRecord = serde_json::from_value(serde_json::json!({
            "uuid":"caller-first","type":"user","timestamp":"2026-09-01T00:00:10Z",
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic"}]}
        }))
        .unwrap();
        store.upsert_records(native, &[input], false).unwrap();
        store
            .record_native_source_locator(
                &SourceCursor {
                    source: SessionSource::Transcript,
                    cursor_key: format!("claude:{}", path.display()),
                    position: 0,
                    updated_at: 1,
                },
                true,
            )
            .unwrap();
        let first = OwnFirst {
            uuid: Some("child-first".into()),
            ts_ms: Some(
                xt_store::timestamp::parse("2026-09-01T00:00:11Z")
                    .unwrap()
                    .1,
            ),
            cwd: None,
            digest: None,
        };
        let mut backlog = BashBacklog::default();
        backlog.scanned.insert(
            path.clone(),
            Scanned {
                generation: source::stat(&path).unwrap(),
                _seq: 2,
                failed: false,
            },
        );
        assert!(
            backlog
                .clear_for(&store, home, "child", "child", &first, 1)
                .unwrap()
        );
        fs::write(&path, b"first\nappended\n").unwrap();
        assert!(
            !backlog
                .clear_for(&store, home, "child", "child", &first, 1)
                .unwrap()
        );
        assert!(backlog.queued.contains(native));
    }

    fn waiting(path: &Path, call: &str, answer: bool) -> Waiting {
        let invocation = Invocation {
            call_id: call.to_owned(),
            index: 0,
            launch_uuid: format!("{call}-launch"),
            launch_ms: 0,
            result: Some((format!("{call}-result"), 10)),
            result_record: Some(format!("{call}-result")),
            end_ms: Some(10),
            cwd: "/tmp/synthetic".to_owned(),
            prompt: "Synthetic prompt".to_owned(),
            prompt_digest: [0; 32],
            named: None,
            answer: answer.then_some([0; 32]),
        };
        let allowance = Allowance::new(1_000_000);
        let held = allowance.reserve(invocation.bytes()).unwrap();
        Waiting {
            caller: "synthetic-caller".to_owned(),
            path: path.to_owned(),
            generation: source::stat(path).unwrap(),
            invocation,
            state: None,
            _held: held,
        }
    }

    #[test]
    fn a_new_history_during_a_paused_final_folder_walk_forces_selection_again() {
        let temp = tempfile::TempDir::new().unwrap();
        let folder = temp.path().join("project");
        fs::create_dir(&folder).unwrap();
        let first = folder.join("first.jsonl");
        fs::write(&first, b"first child").unwrap();
        let mut state = CheckState::new(vec![folder.clone()]);
        state.listed.push(Listed {
            path: first.clone(),
            generation: source::stat(&first).unwrap(),
            first: First::NoInput,
        });
        let mut one_entry = Budget::new(1, Duration::from_secs(2), None).with_entries(1);
        assert_eq!(verify_folders(&mut state, &mut one_entry), Verify::Paused);
        assert!(state.verify_listing.is_some());

        // An exact second possible child can appear inside the already-open
        // folder, even when no new project folder appears at the root.
        fs::write(folder.join("second.jsonl"), b"second matching child").unwrap();
        let mut next = Budget::new(1, Duration::from_secs(2), None).with_entries(1);
        assert_eq!(verify_folders(&mut state, &mut next), Verify::Changed);

        // A fresh complete listing sees both files and cannot approve the
        // old one-file selection.
        let mut fresh = CheckState::new(vec![folder]);
        fresh.listed = state.listed;
        let mut ample = Budget::new(1, Duration::from_secs(2), None).with_entries(10);
        assert_eq!(verify_folders(&mut fresh, &mut ample), Verify::Changed);
    }

    #[test]
    fn a_new_project_during_a_paused_final_folder_walk_invalidates_uniqueness() {
        let temp = tempfile::TempDir::new().unwrap();
        let projects = temp.path().join("projects");
        let folder = projects.join("first-project");
        fs::create_dir_all(&folder).unwrap();
        let child = folder.join("child.jsonl");
        fs::write(&child, b"first child").unwrap();
        let mut state = CheckState::new(vec![folder.clone()]);
        state.projects_stamp = directory_stamp(&projects);
        state.projects_done = true;
        state.listed.push(Listed {
            path: child,
            generation: source::stat(&folder.join("child.jsonl")).unwrap(),
            first: First::NoInput,
        });
        let mut one_entry = Budget::new(1, Duration::from_secs(2), None).with_entries(1);
        assert_eq!(verify_folders(&mut state, &mut one_entry), Verify::Paused);

        let other = projects.join("second-project");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("child.jsonl"), b"same child name").unwrap();
        let mut ample = Budget::new(1, Duration::from_secs(2), None).with_entries(10);
        assert_eq!(verify_folders(&mut state, &mut ample), Verify::Complete);
        assert_eq!(project_walk_current(&state, &projects), Verify::Changed);
    }

    #[test]
    fn a_persistently_unreadable_folder_waits_for_a_later_import() {
        let temp = tempfile::TempDir::new().unwrap();
        let not_a_folder = temp.path().join("not-a-folder");
        fs::write(&not_a_folder, b"plain file").unwrap();
        let mut state = CheckState::new(vec![not_a_folder]);
        for _ in 0..2 {
            let mut budget = Budget::new(1, Duration::from_secs(2), None).with_entries(1);
            assert_eq!(verify_folders(&mut state, &mut budget), Verify::Unreadable);
            assert_eq!(
                budget.entries, 0,
                "unreadable input must not spin through entries"
            );
        }
    }

    #[test]
    fn a_caller_rewritten_while_final_folder_verification_is_paused_is_retried() {
        let temp = tempfile::TempDir::new().unwrap();
        let caller = temp.path().join("caller.jsonl");
        fs::write(&caller, b"caller A").unwrap();
        let child_folder = temp.path().join("child-project");
        fs::create_dir(&child_folder).unwrap();
        let child = child_folder.join("child.jsonl");
        fs::write(&child, b"child").unwrap();
        let launch = waiting(&caller, "launch", true);
        let mut state = CheckState::new(vec![child_folder]);
        state.listed.push(Listed {
            path: child.clone(),
            generation: source::stat(&child).unwrap(),
            first: First::NoInput,
        });
        let mut one_entry = Budget::new(1, Duration::from_secs(2), None).with_entries(1);
        assert_eq!(verify_folders(&mut state, &mut one_entry), Verify::Paused);
        std::thread::sleep(Duration::from_millis(5));
        fs::write(&caller, b"caller B").unwrap();
        assert_ne!(source::stat(&caller).unwrap(), launch.generation);
        let mut ample = Budget::new(1, Duration::from_secs(2), None).with_entries(10);
        assert_eq!(verify_folders(&mut state, &mut ample), Verify::Complete);

        let mut backlog = BashBacklog::default();
        backlog.read.insert(caller.clone(), launch.generation);
        assert!(!caller_current(&mut backlog, &launch));
        assert_eq!(
            backlog.queue.front().map(String::as_str),
            Some("synthetic-caller")
        );
        assert!(!backlog.read.contains_key(&caller));
    }

    #[test]
    fn an_accepted_launch_drops_finished_io_and_capacity_defers_only_its_caller() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("caller.jsonl");
        fs::write(&path, b"caller").unwrap();
        let mut backlog = BashBacklog {
            test_accepted_limit: Some(1),
            ..BashBacklog::default()
        };
        let mut first = waiting(&path, "first", true);
        let mut state = CheckState::new(vec![temp.path().to_owned()]);
        state.listing = Some(fs::read_dir(temp.path()).unwrap());
        first.state = Some(state);
        assert!(matches!(retain_accepted(&mut backlog, first), Intake::Kept));
        assert!(backlog.accepted[0].state.is_none());

        backlog
            .read
            .insert(path.clone(), source::stat(&path).unwrap());
        assert!(matches!(
            retain_accepted(&mut backlog, waiting(&path, "second", true)),
            Intake::Uncached
        ));
        assert_eq!(backlog.accepted.len(), 1);
        assert!(!backlog.read.contains_key(&path));
    }

    #[test]
    fn a_full_waiting_list_keeps_an_accepted_recheck_in_the_same_backlog() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("caller.jsonl");
        fs::write(&path, b"caller").unwrap();
        let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
        let mut backlog = BashBacklog {
            test_waiting_limit: Some(1),
            ..BashBacklog::default()
        };
        backlog.waiting.push(waiting(&path, "waiting", true));
        backlog.accepted.push(waiting(&path, "accepted", true));
        backlog.recheck = true;
        backlog.recheck_accepted = true;
        let limits = LaunchLimits {
            max_pass_entries: 1,
            ..LaunchLimits::default()
        };
        continue_claude_bash(&mut store, temp.path(), &mut backlog, &limits, None, 20).unwrap();
        assert_eq!((backlog.waiting.len(), backlog.accepted.len()), (1, 1));
        assert!(!backlog.recheck && backlog.recheck_accepted);

        // No new import means no new possible child and no useful retry.
        let again =
            continue_claude_bash(&mut store, temp.path(), &mut backlog, &limits, None, 20).unwrap();
        assert_eq!(again, BashProgress::default());
        assert_eq!((backlog.waiting.len(), backlog.accepted.len()), (1, 1));

        backlog.waiting[0].invocation.answer = None;
        backlog.add_sessions(["new-synthetic-child"]);
        continue_claude_bash(&mut store, temp.path(), &mut backlog, &limits, None, 20).unwrap();
        assert_eq!((backlog.waiting.len(), backlog.accepted.len()), (1, 0));
        assert_eq!(backlog.waiting[0].invocation.call_id, "accepted");
    }
}
