//! Automatic parent links for Claude sessions a Codex agent created with a
//! recorded `claude -p --session-id` launch.
//!
//! A Codex agent that runs one create-mode Claude print command naming its
//! new session with `--session-id` (literal or from the immediately preceding
//! owned cell's one-use printed binding), and whose own operation the
//! host acknowledged with a process result (any exit code, or a running
//! process), created that session once it exists as a fresh saved session
//! whose first input is that command's literal prompt — or, for a prompt read
//! from a redirected file, which is never opened, any one text. Whether the
//! job later succeeds, fails or is ever polled again does not matter. This
//! module finds such launches in the Codex thread's own history and, once
//! the named session is indexed and proved, records first that it is an
//! agent's child (a child fact, which needs no parent) and then, when the
//! thread is exactly one indexed Codex user session, links it to that
//! parent. The link is shown as the child's parent; like every creation
//! relation, it also marks the child's user messages as sent by the agent
//! (the Human input view's existing rule). A child fact alone does neither.
//!
//! The same scan finds a Codex agent's literal fresh `codex exec --json`
//! launch ([`codex_cli`]), whose new Codex thread is the one its own first
//! process result printed as `thread.started`, even while the run goes on.
//! Such a child must be a fresh saved Codex session: exactly one indexed
//! Codex user session, whose one recorded original rollout opens with its own
//! header, started by `exec` after the launch, naming no parent, fork or
//! inherited history ([`resolve`]). Its child fact and relation rest on the
//! same chain, checks and guarded writes, as `codex_cli_launch` evidence.
//!
//! Work is kept in a [`LaunchBacklog`] and done in bounded passes
//! ([`continue_claude_launches_into`]) that the watcher runs after its quiet
//! period, without waiting for another source event:
//!
//! 1. A name-only census of the Codex history root ([`group`]), kept across
//!    passes until complete: the index records only a thread's original
//!    rollout, not its continuations. A census that could not read an entry
//!    is taken again later; no history is used from an incomplete census.
//! 2. Threads to look at: every Codex thread the census found once per run
//!    (the startup sweep), indexed or not, each thread a Codex scan imported, the parents of
//!    launches naming a Claude session a Claude scan imported, and the
//!    parents of launches waiting for a timed retry.
//! 3. A thread whose published validation is current — made by this
//!    version ([`CLAUDE_LAUNCH_VALIDATION_VERSION`]), and the census finds
//!    exactly its member files, each exactly its published generation — is
//!    not read again. Otherwise the thread is marked pending (nothing of it
//!    is linked) and its whole history is validated: every file's opening
//!    line and base cutoff ([`group::Planner`]), then every file read whole
//!    ([`scan`]) as the generation it was planned from, then, across all
//!    files, every call identifier a launch's acknowledgment was followed
//!    through must occur exactly once.
//!    The launches found are staged and published at once with the member
//!    set and verdict, only if nothing changed meanwhile. A file whose last
//!    line is still being written holds the thread pending: it is not read
//!    again until one of its files changes.
//! 4. Its launches not yet linked are matched with their children
//!    ([`resolve`]). Right before a link, the whole history is checked again
//!    against the census and every member's generation, and the store writes
//!    it only if the published validation and the launch are still exactly
//!    what was checked. A child imported after its launch was passed over
//!    has that launch looked at again before the thread's launches are done.
//!
//! Every read of a pass — history, opening lines, cutoffs, recorded lines,
//! child transcripts, directory entries — asks the pass's one budget first
//! and is charged what it read; a pass that ends mid-read keeps the read, and
//! the thread goes behind every other queued thread. Retained buffers are
//! reserved at their actual capacity before they grow: at most
//! [`MAX_THREADS`] threads hold work at once, each within
//! [`LaunchLimits::thread_memory`], idle member facts within
//! [`LaunchLimits::cache_memory`], and the pass's read chunk and store pages
//! within [`LaunchLimits::pass_memory`] — 256 MiB in all by default. Map
//! entries are bounded apart; the census is bounded by its entries.
//!
//! Only identifiers, positions, ordinals, generations, closed labels and a
//! digest of launch inputs are stored; no prompt, command, output or path.

mod ack;
pub mod bash;
mod cell;
pub mod check;
mod codex_cli;
mod command;
pub(super) mod group;
mod options;
mod resolve;
mod rows;
mod scan;
mod shell;
mod source;

use super::readers_cli::CancelToken;
use super::session_source::host_roots;
use group::{Census, CensusFile, Plan, Planner, Refusal, Segment};
pub(in crate::native) use resolve::fresh_exec_opened;
use resolve::{Outcome, Resolve};
use scan::{Counts, MemberFacts, ScanBounds, ScanError, SegmentScan, Step};
use source::{Allowance, Budget, Reserved, Unread};
use std::{
    collections::{HashMap, HashSet, VecDeque, hash_map::RandomState},
    path::{Path, PathBuf},
    time::Duration,
};
use xt_store::{
    Host, Store,
    child_fact::ChildFactDisposition,
    claude_launch::{
        Allocation, CandidateRow, ChildState, GroupStatus, LaunchCandidate, LaunchKey,
        MAX_CANDIDATE_ROWS, MemberRecord, SegmentGeneration,
    },
    creation::{CreationAbstention, CreationDisposition},
};

/// Version of this scan's history validation, kept with each published
/// validation. A thread validated by another version is read again once;
/// it moves apart from the creation evidence version its proofs carry.
/// Version 10 also keeps the launches a history started with no first
/// result yet ([`scan::Unfinished`]), which hold a session they may have
/// created unchecked, and reads a spawned helper whose inherited context
/// starts with the header it copied, so every thread is read again once. Version 9 also
/// finds fresh `codex exec --json` launches of Codex
/// children, so every thread is read again once; version 8 reads spawned
/// helper threads with no inherited-context marker,
/// approval reviewers, and helper forks whose marker says which rows were
/// copied, so threads refused for those headers are read again; version 7
/// reads launch options through the shared option map (creation
/// version 6), so threads refused for an output format, verbose logging or
/// standard-error routing are read again; version 6 adds the one-use printed
/// UUID binding; version 5 found the literal launch forms of creation
/// version 4.
pub const CLAUDE_LAUNCH_VALIDATION_VERSION: u32 = 10;
// The display check stores the scan version it ran under.
const _: () =
    assert!(CLAUDE_LAUNCH_VALIDATION_VERSION == xt_store::child_check::LAUNCH_VALIDATION_VERSION);

/// Threads that may hold work in memory at once; another waits its turn.
pub const MAX_THREADS: usize = 2;
/// Found launches one thread's history may hold before it is refused.
pub const MAX_FOUND_CHAINS: usize = 100_000;
/// Call-identifier references its found launches may hold.
pub const MAX_CHAIN_IDS: usize = 1_000_000;
/// Launches followed at once in one member read.
pub const MAX_OPEN_CHAINS: usize = 64;

/// The ceilings of one pass and of the memory the work keeps. Tests lower
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchLimits {
    /// Source bytes one pass may read, over everything it reads.
    pub max_pass_bytes: u64,
    /// How long one pass may run.
    pub deadline: Duration,
    /// Longest history row; a longer one makes its file invalid.
    pub max_line: usize,
    pub max_window_bytes: u64,
    pub max_window_lines: usize,
    pub max_child_bytes: u64,
    pub max_child_lines: usize,
    /// Directory entries one pass may look at, over the census and every
    /// check that no other project holds a child's file.
    pub max_pass_entries: u64,
    /// Retained buffers one thread's work may hold.
    pub thread_memory: usize,
    /// Call-count map entries one thread's work may hold.
    pub thread_map_entries: usize,
    /// Retained buffers and map entries idle cached member facts may hold.
    pub cache_memory: usize,
    pub cache_map_entries: usize,
    /// The pass's read chunk and store pages.
    pub pass_memory: usize,
}

impl Default for LaunchLimits {
    fn default() -> Self {
        Self {
            max_pass_bytes: 256 << 20,
            deadline: Duration::from_secs(5),
            max_line: 16 << 20,
            max_window_bytes: 64 << 20,
            max_window_lines: 200_000,
            max_child_bytes: 256 << 20,
            max_child_lines: 1_000_000,
            max_pass_entries: 100_000,
            // 2 threads × 112 MiB + 24 MiB cache + 8 MiB pass = 256 MiB of
            // buffers.
            thread_memory: 112 << 20,
            thread_map_entries: 1_500_000,
            cache_memory: 24 << 20,
            // 2 × 1.5 M + 1 M = 4 M map entries.
            cache_map_entries: 1_000_000,
            pass_memory: 8 << 20,
        }
    }
}

impl LaunchLimits {
    /// Launch limits of a pass that may read `bytes` more within `time`.
    pub fn remaining(bytes: u64, time: Duration) -> Self {
        Self {
            max_pass_bytes: bytes,
            deadline: time,
            ..Self::default()
        }
    }
}

/// Threads a pass sweeps into its queue at a time.
const SWEEP_PAGE: usize = 256;
/// Imported children looked up per pass.
const CHILD_PAGE: usize = 64;
/// Retry candidates looked up per pass.
const RETRY_PAGE: usize = 256;
/// Launches of one imported child read at a time.
const CHILD_ROWS: usize = 256;
/// The most one stored launch row holds: its fixed part, three 36-character
/// identifiers, two call identifiers of at most 256 and a handle of at most
/// 32, with room to spare.
const ROW_BOUND: usize = std::mem::size_of::<CandidateRow>() + 2048;
/// Transcripts remembered as unfinished before the oldest are forgotten
/// (a forgotten one is only read again).
const MAX_UNFINISHED: usize = 4096;

/// Idle member facts kept for the next validation of their thread, so only
/// changed files are read again within a run.
#[derive(Debug)]
struct Cache {
    facts: HashMap<(String, String), Box<MemberFacts>>,
    order: VecDeque<(String, String)>,
    memory: Allowance,
    entries: Allowance,
}

impl Cache {
    fn new(limits: &LaunchLimits) -> Self {
        Self {
            facts: HashMap::new(),
            order: VecDeque::new(),
            memory: Allowance::new(limits.cache_memory),
            entries: Allowance::new(limits.cache_map_entries),
        }
    }

    /// Keep `facts` idle, evicting the oldest until they fit; dropped when
    /// they alone do not.
    fn keep(&mut self, thread: &str, facts: Box<MemberFacts>) {
        let key = (thread.to_owned(), facts.rollout.clone());
        self.take(&key.0, &key.1);
        let mut facts = facts;
        loop {
            match rehome(facts, &self.memory, &self.entries) {
                Ok(kept) => {
                    self.order.push_back(key.clone());
                    self.facts.insert(key, kept);
                    return;
                }
                Err(back) => {
                    let Some(oldest) = self.order.pop_front() else {
                        return;
                    };
                    self.facts.remove(&oldest);
                    facts = back;
                }
            }
        }
    }

    fn take(&mut self, thread: &str, rollout: &str) -> Option<Box<MemberFacts>> {
        let key = (thread.to_owned(), rollout.to_owned());
        self.order.retain(|kept| *kept != key);
        self.facts.remove(&key)
    }
}

/// Move facts' reservations to other allowances, or give them back.
fn rehome(
    mut facts: Box<MemberFacts>,
    memory: &Allowance,
    entries: &Allowance,
) -> Result<Box<MemberFacts>, Box<MemberFacts>> {
    let (Ok(bytes), Ok(count)) = (
        memory.reserve(facts.bytes.bytes()),
        entries.reserve(facts.entries.bytes()),
    ) else {
        return Err(facts);
    };
    facts.bytes = bytes;
    facts.entries = count;
    Ok(facts)
}

/// One thread's work in progress, kept in memory between passes.
#[derive(Debug)]
struct ThreadWork {
    memory: Allowance,
    entries: Allowance,
    /// The census the work was planned from.
    census_epoch: u64,
    phase: Phase,
}

// Member facts stay boxed as they move between an attempt and the cache.
#[allow(clippy::vec_box)]
#[derive(Debug)]
enum Phase {
    Planning {
        allocation: Allocation,
        planner: Box<Planner>,
    },
    Reading {
        allocation: Allocation,
        plan: Plan,
        facts: Vec<Box<MemberFacts>>,
        /// The list of facts' slots.
        slots: Reserved,
        scan: Option<Box<SegmentScan>>,
    },
    Staging {
        allocation: Allocation,
        plan: Plan,
        facts: Vec<Box<MemberFacts>>,
        _slots: Reserved,
        status: GroupStatus,
        members: Vec<MemberRecord>,
        candidates: Vec<LaunchCandidate>,
        /// The members and candidates, copied from the facts.
        _copies: Reserved,
        staged: usize,
    },
    Resolving {
        revision: i64,
        members: Vec<MemberRecord>,
        paths: HashMap<String, PathBuf>,
        /// The members and paths.
        _held: Reserved,
        cursor: Option<LaunchKey>,
        current: Option<Box<resolve::Resolution>>,
        /// A child of a launch at or before the cursor was imported: the
        /// launches are looked at once more from the first.
        revisit: bool,
    },
}

/// Launch work not yet done. It lives with whoever scans, in memory: a
/// restarted worker sweeps every indexed thread again, which costs a stat per
/// member file of a thread whose validation is current, and validates an
/// unfinished thread again from its first byte.
#[derive(Debug)]
pub struct LaunchBacklog {
    /// One key for every call-identifier hash of this run.
    key: RandomState,
    census: Option<Census>,
    census_epoch: u64,
    /// A scan imported threads after the census was taken.
    census_dirty: bool,
    threads: VecDeque<String>,
    queued: HashSet<String>,
    children: VecDeque<String>,
    children_queued: HashSet<String>,
    /// Where the front child's launches are read up to.
    child_cursor: Option<LaunchKey>,
    /// The startup sweep's position among the census's threads.
    sweep: Option<Option<String>>,
    /// Launches waiting for a timed retry, and where their pages are.
    retry: bool,
    retry_cursor: Option<LaunchKey>,
    work: HashMap<String, ThreadWork>,
    cache: Option<Cache>,
    /// The pass's read chunk and store pages.
    pass_memory: Option<Allowance>,
    /// Threads with a file whose last line was still being written, as the
    /// files were then: not read again until one changes.
    held: HashMap<String, Vec<MemberRecord>>,
    /// Child transcripts whose last line was still being written, as they
    /// were then.
    unfinished: HashMap<PathBuf, SegmentGeneration>,
    /// Threads whose validation this run published or found current, with
    /// the member files it covers and how it ended: what the display check
    /// compares the census with, and why an invalid one is invalid.
    confirmed: HashMap<String, Confirmed>,
    /// The launches each valid validation found started with no first result
    /// yet, by thread, kept while a newer validation of it is read.
    starts: HashMap<String, Vec<scan::Unfinished>>,
    /// A confirmed history changed other than by growing since.
    captured_changed: bool,
    /// Launches set aside without a decision by an earlier run were looked
    /// at again, once, by this one.
    recovered: bool,
}

impl Default for LaunchBacklog {
    fn default() -> Self {
        Self {
            key: RandomState::new(),
            census: None,
            census_epoch: 0,
            census_dirty: false,
            threads: VecDeque::new(),
            queued: HashSet::new(),
            children: VecDeque::new(),
            children_queued: HashSet::new(),
            child_cursor: None,
            sweep: None,
            retry: false,
            retry_cursor: None,
            work: HashMap::new(),
            cache: None,
            pass_memory: None,
            held: HashMap::new(),
            unfinished: HashMap::new(),
            confirmed: HashMap::new(),
            starts: HashMap::new(),
            captured_changed: false,
            recovered: false,
        }
    }
}

impl LaunchBacklog {
    /// The backlog a watcher starts with: a sweep over every indexed thread,
    /// and a look at every launch waiting for a retry.
    pub fn starting() -> Self {
        Self {
            sweep: Some(None),
            retry: true,
            ..Self::default()
        }
    }

    /// Codex threads a scan imported: validated again after a fresh census.
    pub fn add_threads<'a>(&mut self, natives: impl IntoIterator<Item = &'a str>) {
        let mut any = false;
        for native in natives {
            any = true;
            self.queue(native);
        }
        if any {
            self.census_dirty = true;
        }
    }

    /// Sessions a scan imported that a launch may name: their launches are
    /// looked at again.
    pub fn add_children<'a>(&mut self, natives: impl IntoIterator<Item = &'a str>) {
        for native in natives {
            if self.children_queued.insert(native.to_owned()) {
                self.children.push_back(native.to_owned());
            }
        }
    }

    fn queue(&mut self, native: &str) {
        if self.queued.insert(native.to_owned()) {
            self.threads.push_back(native.to_owned());
        }
    }

    /// Whether a later pass has anything to do.
    pub fn pending(&self) -> bool {
        !self.threads.is_empty()
            || !self.children.is_empty()
            || self.sweep.is_some()
            || self.retry
            || self.census_dirty
            || self.census.as_ref().is_some_and(|census| !census.usable())
    }

    /// Whether the census is complete, whole and not superseded.
    fn census_current(&self) -> bool {
        !self.census_dirty && self.census.as_ref().is_some_and(Census::usable)
    }

    /// The number of the census taken last; a census asked for later has a
    /// higher one.
    pub(crate) fn census_epoch(&self) -> u64 {
        self.census_epoch
    }

    /// Take the census again before the next threads are read: a session
    /// born since the last one may have been created by a history it did not
    /// find.
    pub(crate) fn renew_census(&mut self) {
        self.census_dirty = true;
    }

    /// Whether every Codex history the current census finds has been read
    /// for launches since census `epoch` — the census in use when a display
    /// check began — and is still exactly what that read covered: a valid or
    /// explicitly unsupported validation of this run, its files unchanged,
    /// with no planning, reading, staging or held read of it in progress. A
    /// thread whose launches are being resolved counts: its launches are
    /// known, and another child's resolution does not bear on this one. A
    /// thread not yet so is queued to be read; one whose read failed is not
    /// queued again until it changes.
    pub(crate) fn discovered_since(&mut self, epoch: u64) -> Discovery {
        if !self.census_current() || self.census_epoch <= epoch {
            return Discovery::Pending;
        }
        let census = self.census.as_ref().expect("current");
        let mut behind = Vec::new();
        let mut failed = false;
        for thread in census.threads() {
            let reading = self.held.contains_key(thread)
                || self
                    .work
                    .get(thread)
                    .is_some_and(|work| !matches!(work.phase, Phase::Resolving { .. }));
            let current = self.confirmed.get(thread).filter(|confirmed| {
                confirmed.epoch > epoch
                    && census
                        .group(thread)
                        .is_some_and(|files| same_files(files, &confirmed.members))
            });
            match current {
                Some(confirmed) if !reading => {
                    failed |= confirmed.outcome == ReadOutcome::Failed;
                }
                _ => behind.push(thread.to_owned()),
            }
        }
        if !behind.is_empty() {
            for thread in &behind {
                self.queue(thread);
            }
            return Discovery::Pending;
        }
        if failed {
            Discovery::Failed
        } else {
            Discovery::Complete
        }
    }

    /// Whether a launch some current history started with no first result
    /// yet may have created the session `child` of `host`: a Claude launch
    /// naming exactly it, or a Codex launch made strictly before a fresh
    /// `exec` thread was opened at `opened_ms`. A launch naming another
    /// session, or made at or after the thread opened, holds nothing here.
    pub(crate) fn unfinished_for(&self, host: Host, child: &str, opened_ms: i64) -> bool {
        let census = self.census.as_ref();
        self.starts
            .iter()
            .filter(|(thread, _)| {
                thread.as_str() != child
                    && census.is_some_and(|census| census.group(thread).is_some())
            })
            .flat_map(|(_, starts)| starts)
            .any(|start| {
                start.host == host
                    && match host {
                        Host::Claude => start.child == child,
                        _ => start.launch_ms < opened_ms,
                    }
            })
    }

    /// Whether a history this display check read since it began changed in
    /// a way other than growing: a file replaced, shortened, added or gone.
    /// Taken once: the checks in flight start again.
    pub(crate) fn take_captured_changed(&mut self) -> bool {
        std::mem::take(&mut self.captured_changed)
    }

    /// Keep what a published or confirmed validation covers, its outcome and
    /// the launches it found with no result yet, for the display check. A
    /// Claude launch naming a session that the thread's previous launches
    /// with no result did not name moves that session's check on.
    fn note_confirmed(
        &mut self,
        store: &mut Store,
        thread: &str,
        members: &[MemberRecord],
        outcome: ReadOutcome,
        starts: Vec<scan::Unfinished>,
    ) -> xt_store::Result<usize> {
        if let Some(before) = self.confirmed.get(thread)
            && !grown(&before.members, members)
        {
            self.captured_changed = true;
        }
        let known = self.starts.get(thread);
        let mut reopened = 0;
        for start in &starts {
            if start.host == Host::Claude
                && !known.is_some_and(|known| {
                    known
                        .iter()
                        .any(|old| old.host == Host::Claude && old.child == start.child)
                })
            {
                reopened += store.invalidate_child_checks_of(Host::Claude, &start.child)?;
            }
        }
        self.confirmed.insert(
            thread.to_owned(),
            Confirmed {
                epoch: self.census_epoch,
                outcome,
                members: members.to_vec(),
            },
        );
        if starts.is_empty() {
            self.starts.remove(thread);
        } else {
            self.starts.insert(thread.to_owned(), starts);
        }
        Ok(reopened)
    }
}

/// Whether one validation's member files only grew from another's: the same
/// files, each the same file at least as long as before.
fn grown(before: &[MemberRecord], after: &[MemberRecord]) -> bool {
    before.len() == after.len()
        && before.iter().all(|old| {
            after.iter().any(|new| {
                new.rollout_id == old.rollout_id
                    && new.generation.device == old.generation.device
                    && new.generation.inode == old.generation.inode
                    && new.generation.length >= old.generation.length
            })
        })
}

/// How a thread's history read for launches ended, as the display check
/// needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadOutcome {
    /// Every file read and checked: its launches are known.
    Valid,
    /// A layout this version does not read for launches, by an explicit
    /// guard: a fork whose header marks no copied rows.
    Unsupported,
    /// Malformed, over its limits, or not readable as one history: whether
    /// it launched anything is not known.
    Failed,
}

/// What a display check knows of the Codex histories that might have
/// launched its session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Discovery {
    /// Some are still to be read, or the census to be taken.
    Pending,
    /// Every one was read; one or more could not be checked.
    Failed,
    /// Every one was read and checked, or is explicitly unsupported.
    Complete,
}

/// A thread's validation as this run confirmed it.
#[derive(Debug)]
struct Confirmed {
    /// The census epoch it was confirmed under.
    epoch: u64,
    outcome: ReadOutcome,
    members: Vec<MemberRecord>,
}

/// What one pass did. Counts only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchProgress {
    /// The census is still being taken, or must be taken again.
    pub census_pending: bool,
    /// Threads finished for now, paused on the budget (work kept), waiting
    /// for a census or a free slot, met a busy or changed source, and held
    /// by a file whose last line is still being written.
    pub threads_done: usize,
    pub threads_paused: usize,
    pub threads_waiting: usize,
    pub threads_busy: usize,
    pub threads_unfinished: usize,
    /// Validations published, as valid and as invalid.
    pub published_valid: usize,
    pub published_invalid: usize,
    /// Completed checks reopened by changed launches or unfinished starts.
    pub reopened: usize,
    /// Threads whose published validation was current and not read.
    pub threads_unchanged: usize,
    /// History files read whole, and member facts reused from memory.
    pub files_read: usize,
    pub files_reused: usize,
    pub launches_found: usize,
    /// Launches with no acknowledgment, or whose followed call identifiers
    /// occur elsewhere.
    pub launches_broken: usize,
    pub linked: usize,
    /// Children proved and recorded as child facts, with or without a parent.
    pub children: usize,
    /// Of those, children whose thread is not exactly one indexed Codex user
    /// session now: known children with no parent.
    pub unparented: usize,
    pub waiting: usize,
    pub retry: usize,
    /// Child checks held by a transcript whose last line is still being
    /// written (also counted as `retry`).
    pub unfinished: usize,
    pub rejected: usize,
    /// Proofs that changed what is stored, counted as each commits.
    pub changed: usize,
    /// Source bytes this pass read, over everything it read.
    pub bytes_read: u64,
    /// Directory entries this pass looked at, over the census and the
    /// checks that no other project holds a child's file.
    pub entries: u64,
    /// Whether the pass moved any work forward.
    pub advanced: bool,
}

/// How one thread's turn ended.
enum Visit {
    Done,
    Paused,
    Waiting,
    Busy,
    /// A file's last line is still being written.
    Held,
}

/// [`continue_claude_launches_into`] with its own progress.
pub fn continue_claude_launches(
    store: &mut Store,
    home: &Path,
    backlog: &mut LaunchBacklog,
    limits: &LaunchLimits,
    cancel: Option<&CancelToken>,
    recorded_at: i64,
) -> xt_store::Result<LaunchProgress> {
    let mut progress = LaunchProgress::default();
    continue_claude_launches_into(
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

/// One bounded pass over the backlog, counting into `progress` as it goes:
/// on a store error, what was committed before it is already counted, and
/// every queued thread and child is still queued.
pub fn continue_claude_launches_into(
    store: &mut Store,
    home: &Path,
    backlog: &mut LaunchBacklog,
    limits: &LaunchLimits,
    cancel: Option<&CancelToken>,
    recorded_at: i64,
    progress: &mut LaunchProgress,
) -> xt_store::Result<()> {
    let mut budget = Budget::new(limits.max_pass_bytes, limits.deadline, cancel)
        .with_entries(limits.max_pass_entries);
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
    progress.entries += budget.entries;
    progress.advanced |= progress.files_read > 0
        || progress.linked > 0
        || progress.rejected > 0
        || progress.published_valid + progress.published_invalid > 0;
    result
}

fn pass(
    store: &mut Store,
    home: &Path,
    backlog: &mut LaunchBacklog,
    limits: &LaunchLimits,
    budget: &mut Budget<'_>,
    recorded_at: i64,
    progress: &mut LaunchProgress,
) -> xt_store::Result<()> {
    if backlog.cache.is_none() {
        backlog.cache = Some(Cache::new(limits));
    }
    let pass_memory = backlog
        .pass_memory
        .get_or_insert_with(|| Allowance::new(limits.pass_memory))
        .clone();
    // The one read chunk a pass holds at a time.
    let Ok(_chunk) = pass_memory.reserve(source::CHUNK as usize) else {
        return Ok(());
    };
    // 1. The census: taken again when a scan imported threads since, or when
    // it could not read everything; kept across passes until complete.
    let broken = backlog.census.as_ref().is_some_and(Census::broken);
    if backlog.census.is_none() || backlog.census_dirty || broken {
        backlog.census = Some(Census::start(&host_roots(Host::Codex, home)));
        backlog.census_epoch += 1;
        backlog.census_dirty = false;
    }
    let census = backlog.census.as_mut().expect("started");
    if !census.complete() {
        let before = census.walked();
        let done = census.step(&mut || !budget.entry());
        progress.advanced |= census.walked() > before && !census.broken();
        if !done {
            progress.census_pending = true;
            return Ok(());
        }
    }
    if !census.usable() {
        progress.census_pending = true;
    }

    // Once a run: launches an earlier run set aside without a decision, or
    // stored before decisions were kept, are looked at again.
    if !backlog.recovered {
        for parent in store.reopen_undecided_claude_launch_children()? {
            backlog.queue(&parent);
        }
        backlog.recovered = true;
    }

    // 2. Imported children, a page at a time: their child-side rejections
    // are looked at again, and their parents queued; a parent already past
    // the launch is asked to look again. A child leaves the queue only once
    // every page of it was read.
    for _ in 0..CHILD_PAGE {
        let Some(child) = backlog.children.front().cloned() else {
            break;
        };
        let Ok(_page) = pass_memory.reserve(CHILD_ROWS * ROW_BOUND) else {
            break;
        };
        if backlog.child_cursor.is_none() {
            store.reopen_claude_launch_child(&child)?;
        }
        let page = store.claude_launch_candidates_for_child(
            &child,
            backlog.child_cursor.as_ref(),
            CHILD_ROWS,
        )?;
        for row in &page {
            if row.child_state != ChildState::Linked {
                let parent = &row.candidate.key.parent_native_session_id;
                backlog.queue(parent);
                // How far its launches were looked at: the one being
                // checked, else the last one decided.
                if let Some(ThreadWork {
                    phase:
                        Phase::Resolving {
                            cursor,
                            current,
                            revisit,
                            ..
                        },
                    ..
                }) = backlog.work.get_mut(parent)
                    && current
                        .as_ref()
                        .map(|check| &check.row().candidate.key)
                        .or(cursor.as_ref())
                        .is_some_and(|reached| row.candidate.key <= *reached)
                {
                    *revisit = true;
                }
            }
        }
        progress.advanced = true;
        if page.len() == CHILD_ROWS {
            backlog.child_cursor = page.last().map(|row| row.candidate.key.clone());
        } else {
            backlog.child_cursor = None;
            backlog.children.pop_front();
            backlog.children_queued.remove(&child);
        }
    }
    // Launches waiting for a timed retry: one page per pass.
    if backlog.retry
        && let Ok(_page) = pass_memory.reserve(RETRY_PAGE * ROW_BOUND)
    {
        let page =
            store.claude_launch_retry_candidates(backlog.retry_cursor.as_ref(), RETRY_PAGE)?;
        for key in &page {
            backlog.queue(&key.parent_native_session_id);
        }
        if page.len() == RETRY_PAGE {
            backlog.retry_cursor = page.last().cloned();
        } else {
            backlog.retry_cursor = None;
            backlog.retry = false;
        }
    }
    // The startup sweep, over a whole census only.
    if backlog.census_current()
        && let Some(cursor) = backlog.sweep.take()
    {
        let census = backlog.census.as_ref().expect("current");
        let page: Vec<String> = census
            .threads()
            .filter(|thread| cursor.as_deref().is_none_or(|after| *thread > after))
            .take(SWEEP_PAGE)
            .map(str::to_owned)
            .collect();
        if let Some(last) = page.last() {
            backlog.sweep = Some(Some(last.clone()));
        }
        for thread in &page {
            backlog.queue(thread);
        }
        progress.advanced |= !page.is_empty();
    }

    // 3-4. Threads in turn, each at most once a pass; one that pauses, waits
    // or meets a busy source goes behind all the others.
    let programs = command::installed_programs(home);
    for _ in 0..backlog.threads.len() {
        if budget.exhausted() {
            break;
        }
        let Some(thread) = backlog.threads.pop_front() else {
            break;
        };
        let visited = visit(
            store,
            home,
            backlog,
            &thread,
            &programs,
            limits,
            budget,
            recorded_at,
            progress,
        );
        match visited {
            Ok(Visit::Done) => {
                backlog.queued.remove(&thread);
                progress.threads_done += 1;
            }
            Ok(Visit::Paused) => {
                backlog.threads.push_back(thread);
                progress.threads_paused += 1;
            }
            Ok(Visit::Waiting) => {
                backlog.threads.push_back(thread);
                progress.threads_waiting += 1;
            }
            Ok(Visit::Busy) => {
                backlog.threads.push_back(thread);
                progress.threads_busy += 1;
            }
            Ok(Visit::Held) => {
                backlog.threads.push_back(thread);
                progress.threads_unfinished += 1;
            }
            Err(error) => {
                // The thread keeps its place and its work.
                backlog.threads.push_front(thread);
                return Err(error);
            }
        }
    }
    backlog.retry |= store.claude_launch_retry_pending()?;
    Ok(())
}

/// Whether the thread's history is exactly `members` now: the census is
/// current and finds exactly these member files, and every one is exactly
/// its generation.
fn history_current(backlog: &LaunchBacklog, thread: &str, members: &[MemberRecord]) -> bool {
    if !backlog.census_current() {
        return false;
    }
    let Some(files) = backlog
        .census
        .as_ref()
        .and_then(|census| census.group(thread))
    else {
        return false;
    };
    same_files(files, members)
}

/// Whether the census files are exactly the members, each at its generation.
fn same_files(files: &[CensusFile], members: &[MemberRecord]) -> bool {
    files.len() == members.len()
        && members.iter().all(|member| {
            files.iter().any(|file| {
                file.rollout == member.rollout_id
                    && source::stat(&file.path).ok() == Some(member.generation)
            })
        })
}

/// One thread's turn: begin its work if it has none, then drive that work
/// within the budget.
#[allow(clippy::too_many_arguments)]
fn visit(
    store: &mut Store,
    home: &Path,
    backlog: &mut LaunchBacklog,
    thread: &str,
    programs: &[PathBuf],
    limits: &LaunchLimits,
    budget: &mut Budget<'_>,
    recorded_at: i64,
    progress: &mut LaunchProgress,
) -> xt_store::Result<Visit> {
    let mut work = match backlog.work.remove(thread) {
        Some(work) => work,
        None => match begin(store, home, backlog, thread, limits, progress)? {
            Ok(work) => work,
            Err(visit) => return Ok(visit),
        },
    };
    let visited = drive(
        store,
        home,
        backlog,
        &mut work,
        thread,
        programs,
        limits,
        budget,
        recorded_at,
        progress,
    );
    match visited {
        Ok(Visit::Paused | Visit::Waiting) | Err(_) => {
            backlog.work.insert(thread.to_owned(), work);
        }
        // Dropped work keeps what it read, idle, for the next attempt: only
        // a file that changed is read again.
        Ok(Visit::Busy | Visit::Held) => stash(backlog, thread, work),
        Ok(Visit::Done) => {}
    }
    visited
}

/// Keep dropped work's member facts idle in the cache.
fn stash(backlog: &mut LaunchBacklog, thread: &str, work: ThreadWork) {
    let facts = match work.phase {
        Phase::Reading { facts, .. } | Phase::Staging { facts, .. } => facts,
        Phase::Planning { .. } | Phase::Resolving { .. } => return,
    };
    let cache = backlog.cache.as_mut().expect("made");
    for read in facts {
        cache.keep(thread, read);
    }
}

impl Phase {
    /// The bytes a phase actually keeps, measured from its buffers, apart
    /// from call-count maps (bounded by entries) and fixed-size parts.
    #[cfg(test)]
    fn actual(&self) -> usize {
        let facts = |facts: &Vec<Box<MemberFacts>>| {
            slots_bytes(facts)
                + facts
                    .iter()
                    .map(|read| MemberFacts::held(&read.rollout, &read.chains, &read.unfinished))
                    .sum::<usize>()
        };
        match self {
            Self::Planning { planner, .. } => planner.actual(),
            Self::Reading {
                plan,
                facts: read,
                scan,
                ..
            } => {
                group::segments_bytes(&plan.segments)
                    + facts(read)
                    + scan.as_ref().map_or(0, |scan| scan.actual())
            }
            Self::Staging {
                plan,
                facts: read,
                members,
                candidates,
                ..
            } => {
                group::segments_bytes(&plan.segments)
                    + facts(read)
                    + resolve::members_bytes(members)
                    + candidates.capacity() * std::mem::size_of::<LaunchCandidate>()
                    + candidates
                        .iter()
                        .map(|candidate| {
                            candidate.key.parent_native_session_id.capacity()
                                + candidate.key.rollout_id.capacity()
                                + candidate.key.launch_call_id.capacity()
                                + candidate.child_native_session_id.capacity()
                                + candidate.acknowledgment_call_id.capacity()
                                + candidate
                                    .process_session_id
                                    .as_ref()
                                    .map_or(0, String::capacity)
                        })
                        .sum::<usize>()
            }
            Self::Resolving {
                members,
                paths,
                current,
                ..
            } => {
                resolve::members_bytes(members)
                    + source::table_actual(
                        paths.capacity(),
                        std::mem::size_of::<(String, PathBuf)>(),
                    )
                    + paths
                        .iter()
                        .map(|(rollout, path)| rollout.capacity() + path.capacity())
                        .sum::<usize>()
                    + current.as_ref().map_or(0, |check| check.actual())
            }
        }
    }
}

/// Bytes the facts' list holds for its slots.
#[allow(clippy::vec_box)]
fn slots_bytes(facts: &Vec<Box<MemberFacts>>) -> usize {
    facts.capacity() * std::mem::size_of::<Box<MemberFacts>>()
}

/// The member records and paths a resolution keeps, reserved from `memory`
/// before they are made.
fn resolving_held(
    memory: &Allowance,
    members: &[MemberRecord],
    files: &[(&str, &Path)],
) -> Result<Reserved, Unread> {
    const PATH_SLOT: usize = std::mem::size_of::<(String, PathBuf)>();
    memory.reserve(
        std::mem::size_of_val(members)
            + members
                .iter()
                .map(|member| member.rollout_id.len())
                .sum::<usize>()
            + source::table_bytes(files.len().max(4) * 2, PATH_SLOT)
            + files
                .iter()
                .map(|(rollout, path)| rollout.len() + path.as_os_str().len())
                .sum::<usize>(),
    )
}

/// Begin a thread's work: nothing when its published validation is current
/// and no launch waits; its launches when only they wait; else a new
/// validation, marked pending before anything is read.
fn begin(
    store: &mut Store,
    home: &Path,
    backlog: &mut LaunchBacklog,
    thread: &str,
    limits: &LaunchLimits,
    progress: &mut LaunchProgress,
) -> xt_store::Result<Result<ThreadWork, Visit>> {
    // A thread is read whether or not the index holds it: the census found
    // its history files, and a child it launched is known without a parent.
    // Whether it is exactly one parent is decided only for a child already
    // proved.
    if !backlog.census_current() {
        return Ok(Err(Visit::Waiting));
    }
    let census = backlog.census.as_ref().expect("current");
    let Some(files) = census.group(thread).map(<[CensusFile]>::to_vec) else {
        return Ok(Err(Visit::Done));
    };
    // A file whose last line was being written: nothing to do until one
    // of the files changes.
    if let Some(held) = backlog.held.get(thread) {
        if same_files(&files, held) {
            return Ok(Err(Visit::Held));
        }
        backlog.held.remove(thread);
    }
    let census = backlog.census.as_ref().expect("current");
    let roots = anchored_roots(census.roots(), home);
    let published = store.claude_launch_group(thread)?;
    let members = store.claude_launch_members(thread)?;
    // A validation that found launches with no result yet is current only
    // while this run still holds them, and an invalid one only while this run
    // knows why it is invalid: a restarted worker reads each again, once.
    let known = backlog
        .confirmed
        .get(thread)
        .map(|confirmed| confirmed.outcome);
    let unchanged = published.as_ref().is_some_and(|group| {
        group.validator_version == CLAUDE_LAUNCH_VALIDATION_VERSION
            && match group.status {
                GroupStatus::Valid => {
                    group.unfinished_starts == 0 || backlog.starts.contains_key(thread)
                }
                GroupStatus::Invalid => known.is_some_and(|outcome| outcome != ReadOutcome::Valid),
                GroupStatus::Pending => false,
            }
    }) && same_files(&files, &members);
    if backlog.work.len() >= MAX_THREADS {
        return Ok(Err(Visit::Waiting));
    }
    let memory = Allowance::new(limits.thread_memory);
    let entries = Allowance::new(limits.thread_map_entries);
    let census_epoch = backlog.census_epoch;
    let work = |phase| ThreadWork {
        memory: memory.clone(),
        entries: entries.clone(),
        census_epoch,
        phase,
    };
    if unchanged {
        let group = published.expect("published");
        progress.threads_unchanged += 1;
        let (outcome, starts) = match group.status {
            GroupStatus::Valid => (
                ReadOutcome::Valid,
                backlog.starts.get(thread).cloned().unwrap_or_default(),
            ),
            _ => (known.unwrap_or(ReadOutcome::Failed), Vec::new()),
        };
        progress.reopened += backlog.note_confirmed(store, thread, &members, outcome, starts)?;
        if group.status == GroupStatus::Invalid
            || store
                .claude_launch_open_candidates(thread, None, 1)?
                .is_empty()
        {
            return Ok(Err(Visit::Done));
        }
        let named: Vec<(&str, &Path)> = files
            .iter()
            .map(|file| (file.rollout.as_str(), file.path.as_path()))
            .collect();
        let Ok(held) = resolving_held(&memory, &members, &named) else {
            return Ok(Err(Visit::Done));
        };
        return Ok(Ok(work(Phase::Resolving {
            revision: group.revision.expect("published"),
            paths: paths(&files),
            members,
            _held: held,
            cursor: None,
            current: None,
            revisit: false,
        })));
    }
    // A replacement validation: pending first, before any slower work.
    let allocation =
        store.begin_claude_launch_validation(thread, CLAUDE_LAUNCH_VALIDATION_VERSION)?;
    let indexed = group::indexed_paths(store, thread)?;
    let held = !store
        .user_sessions_with_native(Host::Codex, thread)?
        .is_empty();
    match Planner::start(
        thread,
        &files,
        &indexed,
        held,
        &roots,
        &memory,
        limits.max_line,
    ) {
        Ok(planner) => Ok(Ok(work(Phase::Planning {
            allocation,
            planner: Box::new(planner),
        }))),
        Err(Refusal::Source(Unread::Changed | Unread::Unreadable | Unread::Missing)) => {
            Ok(Err(Visit::Busy))
        }
        // Refused from names and stats alone: nothing was read.
        Err(why) => Ok(Err(refuse(
            store,
            backlog,
            thread,
            allocation,
            &[],
            why.outcome(),
            progress,
        )?)),
    }
}

/// The generations a plan was made from, by rollout.
fn planned(segments: &[Segment]) -> Vec<(String, SegmentGeneration)> {
    segments
        .iter()
        .map(|segment| (segment.rollout.clone(), segment.generation))
        .collect()
}

/// Publish a history that could not be validated as invalid, so it is not
/// read again until a file changes: `Done`. Only against a current, complete
/// census in which every file whose content the refusal rests on
/// (`observed`) is still present as the generation it was read as; else
/// nothing is published and the history is planned again: `Busy`. Files not
/// read are recorded as they are now. `outcome` says, for the display check,
/// whether an explicit guard refused a layout this version does not read or
/// the history could not be checked.
#[allow(clippy::too_many_arguments)]
fn refuse(
    store: &mut Store,
    backlog: &mut LaunchBacklog,
    thread: &str,
    allocation: Allocation,
    observed: &[(String, SegmentGeneration)],
    outcome: ReadOutcome,
    progress: &mut LaunchProgress,
) -> xt_store::Result<Visit> {
    let files = census_files(backlog, thread);
    if !backlog.census_current()
        || files.is_empty()
        || !observed
            .iter()
            .all(|(rollout, _)| files.iter().any(|file| file.rollout == *rollout))
    {
        return Ok(Visit::Busy);
    }
    let mut members = Vec::new();
    for file in &files {
        let Ok(generation) = source::stat(&file.path) else {
            return Ok(Visit::Busy);
        };
        if observed
            .iter()
            .any(|(rollout, read)| *rollout == file.rollout && *read != generation)
        {
            return Ok(Visit::Busy);
        }
        members.push(MemberRecord {
            rollout_id: file.rollout.clone(),
            generation,
            scanned_length: 0,
            valid: false,
        });
    }
    let published = store.publish_claude_launch_validation(
        thread,
        allocation,
        GroupStatus::Invalid,
        &members,
        0,
    )?;
    progress.published_invalid += usize::from(published);
    if published {
        progress.reopened +=
            backlog.note_confirmed(store, thread, &members, outcome, Vec::new())?;
    }
    Ok(if published { Visit::Done } else { Visit::Busy })
}

/// Drive one thread's work through its phases within the budget.
#[allow(clippy::too_many_arguments)]
fn drive(
    store: &mut Store,
    home: &Path,
    backlog: &mut LaunchBacklog,
    work: &mut ThreadWork,
    thread: &str,
    programs: &[PathBuf],
    limits: &LaunchLimits,
    budget: &mut Budget<'_>,
    recorded_at: i64,
    progress: &mut LaunchProgress,
) -> xt_store::Result<Visit> {
    let bounds = ScanBounds {
        max_line: limits.max_line,
        max_window_bytes: limits.max_window_bytes,
        max_window_lines: limits.max_window_lines,
        max_open: MAX_OPEN_CHAINS,
    };
    loop {
        match &mut work.phase {
            Phase::Planning {
                allocation,
                planner,
            } => {
                let allocation = *allocation;
                match planner.advance(budget) {
                    Ok(None) => return Ok(Visit::Paused),
                    Ok(Some(plan)) => {
                        // Members' facts of exactly the planned generation are
                        // reused from memory.
                        let cache = backlog.cache.as_mut().expect("made");
                        let Ok(mut slots) = work
                            .memory
                            .reserve(plan.segments.len() * std::mem::size_of::<Box<MemberFacts>>())
                        else {
                            return refuse(
                                store,
                                backlog,
                                thread,
                                allocation,
                                &planned(&plan.segments),
                                ReadOutcome::Failed,
                                progress,
                            );
                        };
                        let mut facts = Vec::with_capacity(plan.segments.len());
                        for segment in &plan.segments {
                            let Some(kept) = cache.take(thread, &segment.rollout) else {
                                continue;
                            };
                            if kept.generation != segment.generation {
                                continue;
                            }
                            if let Ok(kept) = rehome(kept, &work.memory, &work.entries) {
                                progress.files_reused += 1;
                                facts.push(kept);
                            }
                        }
                        let _ = slots.resize(slots_bytes(&facts));
                        work.phase = Phase::Reading {
                            allocation,
                            plan,
                            facts,
                            slots,
                            scan: None,
                        };
                    }
                    Err(Refusal::Source(
                        Unread::Changed | Unread::Unreadable | Unread::Missing,
                    )) => {
                        return Ok(Visit::Busy);
                    }
                    Err(why) => {
                        return refuse(
                            store,
                            backlog,
                            thread,
                            allocation,
                            &planner.observed(),
                            why.outcome(),
                            progress,
                        );
                    }
                }
            }
            Phase::Reading {
                allocation,
                plan,
                facts,
                slots,
                scan,
            } => {
                let allocation = *allocation;
                if let Some(active) = scan.as_mut() {
                    let before = budget.spent;
                    let step = active.advance(budget, programs, &bounds);
                    progress.advanced |= budget.spent > before;
                    match step {
                        Ok(Step::Paused) => return Ok(Visit::Paused),
                        Ok(Step::Done(read)) => {
                            progress.files_read += 1;
                            progress.launches_broken += read.broken;
                            if slots
                                .resize(slots_bytes(facts).max(
                                    (facts.len() + 1) * std::mem::size_of::<Box<MemberFacts>>(),
                                ))
                                .is_err()
                            {
                                return refuse(
                                    store,
                                    backlog,
                                    thread,
                                    allocation,
                                    &planned(&plan.segments),
                                    ReadOutcome::Failed,
                                    progress,
                                );
                            }
                            facts.push(read);
                            let _ = slots.resize(slots_bytes(facts));
                            *scan = None;
                        }
                        Ok(Step::Unfinished) => {
                            // Held as planned until one of its files changes.
                            progress.files_read += 1;
                            let held = plan
                                .segments
                                .iter()
                                .map(|segment| MemberRecord {
                                    rollout_id: segment.rollout.clone(),
                                    generation: segment.generation,
                                    scanned_length: 0,
                                    valid: true,
                                })
                                .collect();
                            backlog.held.insert(thread.to_owned(), held);
                            *scan = None;
                            return Ok(Visit::Held);
                        }
                        Err(ScanError::Source(_)) => {
                            *scan = None;
                            return Ok(Visit::Busy);
                        }
                        Err(ScanError::OverLimit) => {
                            return refuse(
                                store,
                                backlog,
                                thread,
                                allocation,
                                &planned(&plan.segments),
                                ReadOutcome::Failed,
                                progress,
                            );
                        }
                    }
                    continue;
                }
                let next = plan
                    .segments
                    .iter()
                    .find(|segment| !facts.iter().any(|read| read.rollout == segment.rollout));
                if let Some(segment) = next {
                    if budget.exhausted() {
                        return Ok(Visit::Paused);
                    }
                    match SegmentScan::start(
                        segment,
                        thread,
                        &backlog.key,
                        &work.memory,
                        &work.entries,
                        limits.max_line,
                    ) {
                        Ok(started) => *scan = Some(Box::new(started)),
                        Err(ScanError::Source(_)) => return Ok(Visit::Busy),
                        Err(ScanError::OverLimit) => {
                            return refuse(
                                store,
                                backlog,
                                thread,
                                allocation,
                                &planned(&plan.segments),
                                ReadOutcome::Failed,
                                progress,
                            );
                        }
                    }
                    continue;
                }
                // Every member read: check the whole history, its copies
                // reserved first.
                let Ok(mut copies) = work.memory.reserve(0) else {
                    return Ok(Visit::Busy);
                };
                let Ok((status, candidates, broken)) =
                    finalize(&plan.segments, facts, thread, &mut copies)
                else {
                    return refuse(
                        store,
                        backlog,
                        thread,
                        allocation,
                        &planned(&plan.segments),
                        ReadOutcome::Failed,
                        progress,
                    );
                };
                progress.launches_broken += broken;
                progress.launches_found += candidates.len();
                let member_bytes = facts.len() * std::mem::size_of::<MemberRecord>()
                    + facts.iter().map(|read| read.rollout.len()).sum::<usize>();
                if copies.grow(member_bytes).is_err() {
                    return refuse(
                        store,
                        backlog,
                        thread,
                        allocation,
                        &planned(&plan.segments),
                        ReadOutcome::Failed,
                        progress,
                    );
                }
                let members = facts
                    .iter()
                    .map(|read| MemberRecord {
                        rollout_id: read.rollout.clone(),
                        generation: read.generation,
                        scanned_length: i64::try_from(read.complete).unwrap_or(i64::MAX),
                        valid: read.valid,
                    })
                    .collect();
                let Ok(empty) = work.memory.reserve(0) else {
                    return Ok(Visit::Busy);
                };
                let plan = Plan {
                    segments: std::mem::take(&mut plan.segments),
                    held: std::mem::replace(&mut plan.held, empty),
                };
                let facts = std::mem::take(facts);
                let Ok(empty) = work.memory.reserve(0) else {
                    return Ok(Visit::Busy);
                };
                let slots = std::mem::replace(slots, empty);
                work.phase = Phase::Staging {
                    allocation,
                    plan,
                    facts,
                    _slots: slots,
                    status,
                    members,
                    candidates,
                    _copies: copies,
                    staged: 0,
                };
            }
            Phase::Staging {
                allocation,
                plan,
                facts,
                status,
                members,
                candidates,
                staged,
                ..
            } => {
                let allocation = *allocation;
                // Staged in bounded chunks; nothing is published yet.
                while *staged < candidates.len() {
                    if budget.out_of_time() {
                        return Ok(Visit::Paused);
                    }
                    let end = (*staged + MAX_CANDIDATE_ROWS).min(candidates.len());
                    #[cfg(test)]
                    tests::fault("stage")?;
                    if !store.stage_claude_launch_candidates(
                        thread,
                        allocation.allocated,
                        &candidates[*staged..end],
                    )? {
                        return Ok(Visit::Busy);
                    }
                    *staged = end;
                }
                // The final check: the census finds exactly these members,
                // each still exactly the generation that was read.
                let files = census_files(backlog, thread);
                if !backlog.census_current() || work.census_epoch != backlog.census_epoch {
                    if !backlog.census_current() {
                        return Ok(Visit::Waiting);
                    }
                    if files.len() != members.len()
                        || !members.iter().all(|member| {
                            files.iter().any(|file| file.rollout == member.rollout_id)
                        })
                    {
                        // A file joined or left the history: validate again.
                        return Ok(Visit::Busy);
                    }
                }
                let still = plan.segments.iter().all(|segment| {
                    members.iter().any(|member| {
                        member.rollout_id == segment.rollout
                            && source::stat(&segment.path).ok() == Some(member.generation)
                    })
                });
                if !still {
                    return Ok(Visit::Busy);
                }
                // The launches still waiting for a first result, of a valid
                // history only: names and times, at most one window's worth
                // per file.
                let starts: Vec<scan::Unfinished> = if *status == GroupStatus::Valid {
                    facts
                        .iter()
                        .flat_map(|read| read.unfinished.iter().cloned())
                        .collect()
                } else {
                    Vec::new()
                };
                #[cfg(test)]
                tests::fault("publish")?;
                let Some(reopened) = store.publish_claude_launch_validation_with_reopened(
                    thread,
                    allocation,
                    *status,
                    members,
                    *staged,
                    starts.len(),
                )?
                else {
                    return Ok(Visit::Busy);
                };
                progress.reopened += reopened;
                match status {
                    GroupStatus::Valid => progress.published_valid += 1,
                    _ => progress.published_invalid += 1,
                }
                let outcome = if *status == GroupStatus::Valid {
                    ReadOutcome::Valid
                } else {
                    ReadOutcome::Failed
                };
                progress.reopened +=
                    backlog.note_confirmed(store, thread, members, outcome, starts)?;
                // Keep the member facts idle for the next validation.
                let cache = backlog.cache.as_mut().expect("made");
                for read in std::mem::take(facts) {
                    cache.keep(thread, read);
                }
                if *status != GroupStatus::Valid {
                    return Ok(Visit::Done);
                }
                let members = std::mem::take(members);
                let named: Vec<(&str, &Path)> = plan
                    .segments
                    .iter()
                    .map(|segment| (segment.rollout.as_str(), segment.path.as_path()))
                    .collect();
                let Ok(held) = resolving_held(&work.memory, &members, &named) else {
                    return Ok(Visit::Busy);
                };
                let paths = plan
                    .segments
                    .iter()
                    .map(|segment| (segment.rollout.clone(), segment.path.clone()))
                    .collect();
                work.phase = Phase::Resolving {
                    revision: allocation.allocated,
                    members,
                    paths,
                    _held: held,
                    cursor: None,
                    current: None,
                    revisit: false,
                };
            }
            Phase::Resolving { .. } => {
                return resolve_launches(
                    store,
                    home,
                    backlog,
                    work,
                    thread,
                    programs,
                    limits,
                    budget,
                    recorded_at,
                    progress,
                );
            }
        }
    }
}

/// Every found launch whose followed call identifiers each occur exactly
/// once as a call and at most once as an output, across every member; the
/// verdict; and how many launches were dropped. The candidates copy the
/// facts' strings: `copies` reserves them first, or the thread's allowance
/// cannot hold them.
fn finalize(
    segments: &[Segment],
    facts: &[Box<MemberFacts>],
    thread: &str,
    copies: &mut Reserved,
) -> Result<(GroupStatus, Vec<LaunchCandidate>, usize), Unread> {
    let invalid = Ok((GroupStatus::Invalid, Vec::new(), 0));
    if facts.len() != segments.len() || facts.iter().any(|read| !read.valid) {
        return invalid;
    }
    let chains: usize = facts.iter().map(|read| read.chains.len()).sum();
    let ids: usize = facts
        .iter()
        .flat_map(|read| &read.chains)
        .map(|chain| chain.ids.len())
        .sum();
    if chains > MAX_FOUND_CHAINS || ids > MAX_CHAIN_IDS {
        return invalid;
    }
    let total = |hash: &u64| {
        facts.iter().fold(Counts::default(), |sum, read| {
            sum.plus(read.counts.get(hash).copied().unwrap_or_default())
        })
    };
    let unique = |chain: &scan::Found| {
        chain.ids.iter().all(|hash| {
            let counts = total(hash);
            counts.calls == 1 && counts.outputs <= 1
        })
    };
    // What the kept candidates copy, reserved before any is made.
    let (mut kept, mut bytes) = (0, 0);
    for read in facts {
        for chain in read.chains.iter().filter(|chain| unique(chain)) {
            kept += 1;
            bytes += thread.len()
                + read.rollout.len()
                + chain.launch_call_id.len()
                + chain.child.len()
                + chain.acknowledgment_call_id.len()
                + chain.handle.as_ref().map_or(0, String::len);
        }
    }
    copies.grow(kept * std::mem::size_of::<LaunchCandidate>() + bytes)?;
    let mut candidates = Vec::with_capacity(kept);
    let mut broken = 0;
    for read in facts {
        for chain in &read.chains {
            if !unique(chain) {
                broken += 1;
                continue;
            }
            candidates.push(LaunchCandidate {
                key: LaunchKey {
                    parent_native_session_id: thread.to_owned(),
                    rollout_id: read.rollout.clone(),
                    launch_call_id: chain.launch_call_id.clone(),
                    launch_operation_index: chain.launch_op,
                },
                child_native_session_id: chain.child.clone(),
                acknowledgment_call_id: chain.acknowledgment_call_id.clone(),
                acknowledgment_operation_index: chain.launch_op,
                process_session_id: chain.handle.clone(),
                launch_offset: i64::try_from(chain.launch_offset).unwrap_or(i64::MAX),
                acknowledgment_offset: i64::try_from(chain.acknowledgment_offset)
                    .unwrap_or(i64::MAX),
                launch_ordinal: chain.launch_ordinal,
                acknowledgment_ordinal: chain.acknowledgment_ordinal,
                binding_call_offset: chain
                    .binding_call_offset
                    .and_then(|at| i64::try_from(at).ok()),
                binding_output_offset: chain
                    .binding_output_offset
                    .and_then(|at| i64::try_from(at).ok()),
                child_host: chain.host,
                launch_check_fingerprint: Some(chain.launch_check_fingerprint.clone()),
            });
        }
    }
    Ok((GroupStatus::Valid, candidates, broken))
}

/// Look at a thread's open launches, one at a time in key order, within the
/// budget; a pass that ends keeps the check in progress and its place.
#[allow(clippy::too_many_arguments)]
fn resolve_launches(
    store: &mut Store,
    home: &Path,
    backlog: &mut LaunchBacklog,
    work: &mut ThreadWork,
    thread: &str,
    programs: &[PathBuf],
    limits: &LaunchLimits,
    budget: &mut Budget<'_>,
    recorded_at: i64,
    progress: &mut LaunchProgress,
) -> xt_store::Result<Visit> {
    let Phase::Resolving {
        revision,
        members,
        paths,
        cursor,
        current,
        revisit,
        ..
    } = &mut work.phase
    else {
        unreachable!("resolving");
    };
    loop {
        let sources = resolve::Sources {
            home,
            programs,
            max_line: limits.max_line,
            max_child_bytes: limits.max_child_bytes,
            max_child_lines: limits.max_child_lines,
            max_window_bytes: limits.max_window_bytes,
            unfinished: &backlog.unfinished,
        };
        // Nothing of a history that changed is linked.
        if !history_current(backlog, thread, members) {
            return Ok(if backlog.census_current() {
                Visit::Busy
            } else {
                Visit::Waiting
            });
        }
        let resumed = current.is_some();
        let (row, step) = if let Some(resolution) = current.take() {
            let row = resolution.row().clone();
            let before = (budget.spent, budget.entries);
            let step = resolve::resume(store, resolution, &sources, &work.memory, budget)?;
            progress.advanced |= (budget.spent, budget.entries) != before;
            (row, step)
        } else {
            if budget.exhausted() {
                return Ok(Visit::Paused);
            }
            let next = {
                let Ok(_page) = backlog
                    .pass_memory
                    .as_ref()
                    .expect("made")
                    .reserve(ROW_BOUND)
                else {
                    return Ok(Visit::Paused);
                };
                store
                    .claude_launch_open_candidates(thread, cursor.as_ref(), 1)?
                    .into_iter()
                    .next()
            };
            let Some(row) = next else {
                // A child imported meanwhile: once more from the first.
                if std::mem::take(revisit) {
                    *cursor = None;
                    continue;
                }
                return Ok(Visit::Done);
            };
            if row.source_revision != *revision {
                return Ok(Visit::Busy);
            }
            let Some(path) = paths.get(&row.candidate.key.rollout_id) else {
                store.reject_claude_launch_source(&row)?;
                *cursor = Some(row.candidate.key.clone());
                continue;
            };
            let step = resolve::begin(store, &row, path, members, &sources, &work.memory)?;
            (row, step)
        };
        match step {
            Resolve::Paused(resolution) => {
                *current = Some(resolution);
                // A check paused mid-read ran out of the pass's bytes,
                // entries or time; one only begun is resumed now.
                if resumed || budget.exhausted() {
                    return Ok(Visit::Paused);
                }
            }
            Resolve::Ready(ready) => {
                // Right before the write: the whole history, again.
                if !history_current(backlog, thread, &ready.members) {
                    store.set_claude_launch_child_state(&ready.row, ChildState::Retry)?;
                    progress.retry += 1;
                    return Ok(Visit::Busy);
                }
                // And the child's transcript as its first input was read:
                // one changed since is checked again from its new content.
                if !ready.transcript_current() {
                    store.set_claude_launch_child_state(&ready.row, ChildState::Retry)?;
                    progress.retry += 1;
                    *cursor = Some(ready.row.candidate.key.clone());
                    continue;
                }
                #[cfg(test)]
                tests::fault("link")?;
                // The child first: that the launch created it needs no parent.
                let Some(facts) = store.record_claude_launch_child_guarded(
                    &ready.fact,
                    &ready.row,
                    &ready.members,
                    recorded_at,
                )?
                else {
                    // The published validation or the launch moved on.
                    progress.retry += 1;
                    return Ok(Visit::Busy);
                };
                progress.changed += facts.changed;
                match facts.dispositions.first() {
                    Some(ChildFactDisposition::Abstained(
                        CreationAbstention::MissingChild | CreationAbstention::FirstInputMismatch,
                    )) => {
                        // The index may not hold yet what the transcript
                        // already does.
                        store.set_claude_launch_child_state(&ready.row, ChildState::Retry)?;
                        progress.retry += 1;
                        *cursor = Some(ready.row.candidate.key.clone());
                        continue;
                    }
                    Some(ChildFactDisposition::Abstained(_)) | None => {
                        store.set_claude_launch_child_state(&ready.row, ChildState::Rejected)?;
                        progress.rejected += 1;
                        *cursor = Some(ready.row.candidate.key.clone());
                        continue;
                    }
                    Some(_) => progress.children += 1,
                }
                let Some(proof) = &ready.proof else {
                    // No single indexed parent now: the child stays known,
                    // and the launch is looked at again on its next import.
                    store.set_claude_launch_child_state(&ready.row, ChildState::Rejected)?;
                    progress.unparented += 1;
                    *cursor = Some(ready.row.candidate.key.clone());
                    continue;
                };
                match store.record_claude_launch_guarded(
                    proof,
                    &ready.row,
                    &ready.members,
                    recorded_at,
                )? {
                    None => {
                        // The published validation or the launch moved on.
                        progress.retry += 1;
                        return Ok(Visit::Busy);
                    }
                    Some(report) => {
                        progress.changed += report.changed;
                        // Only a check whose outcome is recorded moves the
                        // cursor: one lost to a failure is checked again.
                        *cursor = Some(ready.row.candidate.key.clone());
                        match report.dispositions.first() {
                            Some(CreationDisposition::Abstained(
                                CreationAbstention::MissingChild
                                | CreationAbstention::FirstInputMismatch,
                            )) => {
                                // The index may not hold yet what the
                                // transcript already does.
                                store
                                    .set_claude_launch_child_state(&ready.row, ChildState::Retry)?;
                                progress.retry += 1;
                            }
                            Some(CreationDisposition::Abstained(_)) | None => {
                                store.set_claude_launch_child_state(
                                    &ready.row,
                                    ChildState::Rejected,
                                )?;
                                progress.rejected += 1;
                            }
                            Some(_) => progress.linked += 1,
                        }
                    }
                }
            }
            Resolve::Finished(outcome) => {
                record(store, &row, outcome, progress)?;
                *cursor = Some(row.candidate.key.clone());
                if outcome == Outcome::Stale {
                    return Ok(Visit::Busy);
                }
            }
            Resolve::Unfinished(transcript, generation) => {
                if backlog.unfinished.len() >= MAX_UNFINISHED {
                    backlog.unfinished.clear();
                }
                backlog.unfinished.insert(transcript, generation);
                record(store, &row, Outcome::Unfinished, progress)?;
                *cursor = Some(row.candidate.key.clone());
            }
        }
    }
}

/// Record what became of one check.
fn record(
    store: &mut Store,
    row: &CandidateRow,
    outcome: Outcome,
    progress: &mut LaunchProgress,
) -> xt_store::Result<()> {
    match outcome {
        Outcome::Waiting => progress.waiting += 1,
        Outcome::Retry | Outcome::Stale => progress.retry += 1,
        Outcome::Unfinished => {
            progress.retry += 1;
            progress.unfinished += 1;
        }
        Outcome::Rejected | Outcome::SourceRejected | Outcome::Deferred | Outcome::Unclear => {
            progress.rejected += 1
        }
    }
    match outcome {
        Outcome::SourceRejected => {
            store.reject_claude_launch_source(row)?;
        }
        // A check the thread's own allowance cannot hold is its own input
        // alone over its limit.
        // Set aside without a decision: not retried on a timer, and the child
        // it names stays checking.
        Outcome::Deferred | Outcome::Unclear => {
            store.defer_claude_launch_child(row)?;
        }
        other => {
            if let Some(state) = other.state()
                && state != row.child_state
            {
                store.set_claude_launch_child_state(row, state)?;
            }
        }
    }
    Ok(())
}

fn census_files(backlog: &LaunchBacklog, thread: &str) -> Vec<CensusFile> {
    backlog
        .census
        .as_ref()
        .and_then(|census| census.group(thread))
        .map(<[CensusFile]>::to_vec)
        .unwrap_or_default()
}

fn paths(files: &[CensusFile]) -> HashMap<String, PathBuf> {
    files
        .iter()
        .map(|file| (file.rollout.clone(), file.path.clone()))
        .collect()
}

/// The census roots and the configured ones, as containment checks take them.
fn anchored_roots(census: &[PathBuf], home: &Path) -> Vec<PathBuf> {
    let mut roots = host_roots(Host::Codex, home);
    for root in census {
        if !roots.contains(root) {
            roots.push(root.clone());
        }
    }
    roots
}

#[cfg(test)]
mod tests;
