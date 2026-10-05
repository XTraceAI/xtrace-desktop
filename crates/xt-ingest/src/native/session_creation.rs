//! Proving, from host-typed structure only, that a Codex thread was spawned by
//! another thread's agent, and recording that as a sub-session relation.
//!
//! Codex writes a spawned thread's parent into the thread's own rollout: its
//! opening `session_meta` carries the thread in `payload.id`,
//! `payload.source.subagent.thread_spawn` with the spawning thread's full
//! `parent_thread_id`, and that same parent in `payload.session_id`. That
//! line, and nothing else, is the evidence; `session_id` alone never is. A
//! `guardian`, `review` or other subagent source names no parent; a
//! `history_base` names the rollout a history continues from, never a
//! parent; a role, task path, nickname, prompt, time or similarity is never
//! read as one. None of them yields a relation.
//!
//! The rollout is the one the index recorded for the thread's root, reached
//! exactly as the title read reaches it: no alias followed, contained in the
//! Codex history root, the only root file recorded for the thread, and the
//! same file — device, inode, length and change time — before and after its
//! opening line is read. Only that complete line is parsed; the rest of the
//! file is never read, and nothing from the line is kept but the two thread
//! identities. Host files are opened for reading only.
//!
//! Two kinds of work share the store's one guarded insertion
//! ([`Store::record_session_creations`]): every Codex thread a native scan
//! indexed, whether or not it added records, since an unchanged transcript
//! can still open with a new or different header; and
//! [`bootstrap_codex_spawns`], a bounded, resumable pass over the Codex
//! sources the index already holds, so a session indexed before relations
//! existed is classified without waiting for it to change. A
//! [`SpawnBacklog`] carries both between bounded passes
//! ([`continue_codex_spawns`]), which the watcher schedules until nothing is
//! left, without waiting for another source event. The backlog a watcher
//! starts with ([`SpawnBacklog::starting`]) also sweeps every indexed Codex
//! root once, from the first, so threads a previous run queued and never read
//! are recovered from the index itself, with or without a reader. None of
//! them resets a checkpoint or replays a transcript.
//!
//! A thread's root rollout is found by an exact, indexed lookup of the
//! locators ending in its name, so a pass costs in proportion to the threads
//! it reads, not to every locator the index holds.

use super::claude_launch::{
    LaunchBacklog, LaunchLimits, LaunchProgress, continue_claude_launches_into,
};
use super::readers_cli::CancelToken;
use super::session_source::{IndexedSource, host_roots};
use super::session_titles::{
    Batch, MAX_CODEX_LOCATORS, Segment, TitleLimits, TypedSpawn, Untitled, codex_contained,
    codex_path, codex_segment, contains, corroborated, full_thread_id, gather, object, typed_spawn,
};
use serde_json::Value;
use std::{
    collections::{HashSet, VecDeque},
    path::Path,
    time::Duration,
};
use xt_store::{
    Error, Host, SessionSource, Store,
    creation::{
        CODEX_THREAD_SPAWN_VERSION, CreationBootstrap, CreationDisposition, CreationEvidence,
        CreationWitness, LOCATOR_TAIL, MAX_CREATION_PROOFS, SessionCreationProof,
    },
};

/// The most Codex threads one [`record_codex_spawns`] call accepts, and one
/// chunk of a [`continue_codex_spawns`] pass probes.
pub const MAX_SPAWN_PROBES: usize = MAX_CREATION_PROOFS;

/// The most source bytes one call may read, over every opening line it reads.
pub const SPAWN_BATCH_BYTES: u64 = 256 * 1024 * 1024;

/// How long one call may read before it stops, leaving the rest for later.
pub const SPAWN_DEADLINE: Duration = Duration::from_secs(5);

/// Locators the bootstrap pass reads from the index at a time.
const BOOTSTRAP_PAGE: usize = 256;

/// The ceilings one call observes: the title read's line bound, a byte
/// budget and a deadline. Tests lower them.
pub fn spawn_limits() -> TitleLimits {
    TitleLimits {
        max_batch_bytes: SPAWN_BATCH_BYTES,
        deadline: SPAWN_DEADLINE,
        ..TitleLimits::default()
    }
}

/// What one Codex thread's verified opening header says about its creation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnOutcome {
    /// The thread's own opening `session_meta` records that the thread with
    /// this full identity spawned it.
    Spawned {
        parent_native_session_id: String,
    },
    /// The opening header is this thread's and records no thread spawn: a
    /// person's session, `exec`, or a `guardian`/`review` subagent.
    NotSpawned,
    /// Native automatic-review origin; a Human assumption, not an exact spawn relation.
    Reviewer {
        parent_native_session_id: String,
    },
    Refused(SpawnRefusal),
}

/// Why a thread's creation could not be read. Each leaves the thread a plain
/// session; none is shown or logged by this module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnRefusal {
    /// Not a full lowercase Codex thread identity.
    InvalidIdentifier,
    /// No indexed user session, or no recorded root rollout, for this thread.
    NotIndexed,
    /// More than one indexed user session holds this thread's identity.
    AmbiguousSession,
    /// The index recorded more locators ending in this thread's root name
    /// than are checked.
    TooManySources,
    /// More than one root rollout is recorded for this thread, or one could
    /// not be looked at safely.
    AmbiguousRoot,
    /// The recorded root could not be read as the file that was observed:
    /// missing, outside the root or an alias, replaced, unreadable, a torn or
    /// overlong opening line, a spent budget, the deadline or a cancel.
    Source(Untitled),
    /// The opening line is not a `session_meta`.
    NoOpeningHeader,
    /// The opening `session_meta` names another thread, or none.
    IdentityMismatch,
    /// A `thread_spawn` whose parent is not one full thread identity.
    MalformedSpawn,
    /// A `thread_spawn` naming the thread itself.
    SelfSpawn,
    /// A well-formed `thread_spawn` whose `payload.session_id` is missing,
    /// null or anything but the parent it names.
    UncorroboratedSpawn,
}

impl SpawnRefusal {
    /// A refusal that says nothing about this thread, only that this call
    /// cannot read further: the thread is left for a later call.
    fn halted(self) -> bool {
        matches!(
            self,
            Self::Source(Untitled::Budget | Untitled::Deadline | Untitled::Cancelled)
        )
    }
}

/// One probed thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnProbe {
    pub native_session_id: String,
    /// The one indexed user session that holds this identity, when there is.
    pub child_session_id: Option<String>,
    pub outcome: SpawnOutcome,
}

/// What one call probed and what the store did with each spawn it proved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnRecording {
    pub probes: Vec<SpawnProbe>,
    /// The store's disposition of each proved spawn, by child native identity,
    /// in probe order.
    pub dispositions: Vec<(String, CreationDisposition)>,
    /// How many threads, from the first, were decided. The rest were reached
    /// by the budget, deadline or a cancel and are left for a later call.
    pub decided: usize,
    /// Proved spawns that changed what is stored
    /// ([`xt_store::creation::CreationReport::changed`]).
    pub changed: usize,
    pub bytes_read: u64,
}

/// Read the opening header of each named Codex thread's recorded root rollout
/// and record every proved spawn through the store's one guarded insertion.
///
/// More than [`MAX_SPAWN_PROBES`] threads are refused before anything is
/// read; a caller with more splits them. The first thread the budget,
/// deadline or a cancel reached is refused, not decided, and nothing after it
/// is recorded: [`SpawnRecording::decided`] says where a later call resumes.
pub fn record_codex_spawns(
    store: &mut Store,
    home: &Path,
    natives: &[&str],
    limits: TitleLimits,
    cancel: Option<&CancelToken>,
    recorded_at: i64,
) -> xt_store::Result<SpawnRecording> {
    let mut batch = Batch::new(limits, cancel);
    record_within(store, home, natives, &mut batch, recorded_at)
}

fn record_within(
    store: &mut Store,
    home: &Path,
    natives: &[&str],
    batch: &mut Batch<'_>,
    recorded_at: i64,
) -> xt_store::Result<SpawnRecording> {
    if natives.len() > MAX_SPAWN_PROBES {
        return Err(Error::InvalidInput(
            "too many Codex threads for one creation probe",
        ));
    }
    let probes = probe(store, home, natives, batch)?;
    let decided = decided(&probes);
    let Recorded {
        dispositions,
        changed,
    } = record(store, &probes[..decided], recorded_at)?;
    Ok(SpawnRecording {
        probes,
        dispositions,
        decided,
        changed,
        bytes_read: batch.bytes_read(),
    })
}

/// How many probes, from the first, were decided: up to the first one this
/// call could not read further for.
fn decided(probes: &[SpawnProbe]) -> usize {
    probes
        .iter()
        .position(|probe| matches!(&probe.outcome, SpawnOutcome::Refused(why) if why.halted()))
        .unwrap_or(probes.len())
}

/// Creation work not yet done: Codex threads a scan indexed whose opening
/// header has not been read since, in the order they arrived, whether the
/// pass over sources indexed before relations existed has finished, and how
/// far a sweep over every indexed root has read.
///
/// It lives with whoever scans. A thread that is still waiting keeps its
/// place when a later scan names it again, so every thread is reached however
/// often scans arrive. The threads are held in memory only: a backlog that
/// ends before they are read (the app quits, say) loses them. The backlog a
/// watcher starts with ([`Self::starting`]) therefore sweeps every Codex root
/// the index holds, from the first key, reading only the index and the roots'
/// opening lines, so what a previous run left is recovered even when the
/// reader cannot run and no source changes. The sweep's position is kept here
/// too; one interrupted starts again from the first key, since the indexed
/// locators themselves are durable. The bootstrap keeps its own progress in
/// the store.
#[derive(Debug, Default)]
pub struct SpawnBacklog {
    threads: VecDeque<String>,
    waiting: HashSet<String>,
    bootstrap_complete: bool,
    sweep: Option<Sweep>,
    /// Automatic Claude launch links ([`super::claude_launch`]): continued
    /// only by the passes a watcher schedules, never inside a scan.
    pub launches: LaunchBacklog,
}

/// How far a sweep over every indexed Codex root has read.
#[derive(Clone, Debug, Default)]
struct Sweep {
    after: Option<String>,
}

impl SpawnBacklog {
    /// The backlog a watcher starts with: nothing queued, and a sweep over
    /// every indexed Codex root pending.
    pub fn starting() -> Self {
        Self {
            sweep: Some(Sweep::default()),
            launches: LaunchBacklog::starting(),
            ..Self::default()
        }
    }

    /// Queue threads whose header is to be read; one already waiting keeps
    /// its place.
    pub fn add<'a>(&mut self, natives: impl IntoIterator<Item = &'a str>) {
        for native in natives {
            if self.waiting.insert(native.to_owned()) {
                self.threads.push_back(native.to_owned());
            }
        }
    }

    /// Threads still waiting.
    pub fn threads(&self) -> usize {
        self.threads.len()
    }

    /// Whether a sweep over every indexed root is still to finish.
    pub fn sweeping(&self) -> bool {
        self.sweep.is_some()
    }

    /// Whether a later pass has anything to do.
    pub fn pending(&self) -> bool {
        !self.threads.is_empty()
            || !self.bootstrap_complete
            || self.sweeping()
            || self.launches.pending()
    }
}

/// What one [`continue_codex_spawns`] pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpawnProgress {
    /// Waiting threads decided and taken off the backlog.
    pub decided: usize,
    /// Proved spawns handed to the store.
    pub spawned: usize,
    /// Proved spawns that changed what is stored: a relation newly recorded,
    /// or an accepted one first withheld as conflicted. Only such a pass has
    /// anything new to show.
    pub changed: usize,
    /// Locators the bootstrap finished.
    pub bootstrapped: usize,
    /// Locators the sweep finished.
    pub swept: usize,
    pub bytes_read: u64,
    /// The pass over Claude launches, when one ran.
    pub launches: Option<LaunchProgress>,
}

impl SpawnProgress {
    /// Whether the pass moved any work forward. One that did not is retried
    /// later, not at once.
    pub fn advanced(&self) -> bool {
        self.decided > 0
            || self.bootstrapped > 0
            || self.swept > 0
            || self
                .launches
                .as_ref()
                .is_some_and(|launches| launches.advanced)
    }
}

/// One bounded pass over the backlog: waiting threads first, in chunks of at
/// most [`MAX_SPAWN_PROBES`], then the bootstrap, then the sweep, all within
/// one budget, deadline and cancel. What the pass could not reach stays in
/// the backlog or in the bootstrap's saved progress for the next pass.
pub fn continue_codex_spawns(
    store: &mut Store,
    home: &Path,
    backlog: &mut SpawnBacklog,
    limits: TitleLimits,
    cancel: Option<&CancelToken>,
    recorded_at: i64,
) -> xt_store::Result<SpawnProgress> {
    let mut progress = SpawnProgress::default();
    continue_codex_spawns_into(
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

/// [`continue_codex_spawns`], counting into `progress` as it goes, so a pass
/// that fails part-way still says what it committed before the failure.
pub fn continue_codex_spawns_into(
    store: &mut Store,
    home: &Path,
    backlog: &mut SpawnBacklog,
    limits: TitleLimits,
    cancel: Option<&CancelToken>,
    recorded_at: i64,
    progress: &mut SpawnProgress,
) -> xt_store::Result<()> {
    continue_within(
        store,
        home,
        backlog,
        limits,
        cancel,
        recorded_at,
        true,
        progress,
    )
}

/// The pass a scan runs over its own threads and the bootstrap. A sweep is
/// left to the watcher's later passes, so it never lengthens a scan.
pub(super) fn continue_scanned_spawns(
    store: &mut Store,
    home: &Path,
    backlog: &mut SpawnBacklog,
    limits: TitleLimits,
    cancel: Option<&CancelToken>,
    recorded_at: i64,
) -> xt_store::Result<SpawnProgress> {
    let mut progress = SpawnProgress::default();
    continue_within(
        store,
        home,
        backlog,
        limits,
        cancel,
        recorded_at,
        false,
        &mut progress,
    )?;
    Ok(progress)
}

#[allow(clippy::too_many_arguments)]
fn continue_within(
    store: &mut Store,
    home: &Path,
    backlog: &mut SpawnBacklog,
    limits: TitleLimits,
    cancel: Option<&CancelToken>,
    recorded_at: i64,
    sweep: bool,
    progress: &mut SpawnProgress,
) -> xt_store::Result<()> {
    let mut batch = Batch::new(limits, cancel);
    while !backlog.threads.is_empty() && batch.stopped().is_none() {
        let chunk: Vec<String> = backlog
            .threads
            .iter()
            .take(MAX_SPAWN_PROBES)
            .cloned()
            .collect();
        let natives: Vec<&str> = chunk.iter().map(String::as_str).collect();
        let recording = record_within(store, home, &natives, &mut batch, recorded_at)?;
        for native in &chunk[..recording.decided] {
            backlog.threads.pop_front();
            backlog.waiting.remove(native);
        }
        progress.decided += recording.decided;
        progress.spawned += recording.dispositions.len();
        progress.changed += recording.changed;
        if recording.decided < chunk.len() {
            break;
        }
    }
    progress.bytes_read = batch.bytes_read();
    if !backlog.bootstrap_complete {
        let mut walk = Walk::default();
        let walked = bootstrap_within(store, home, &mut batch, recorded_at, &mut walk);
        progress.spawned += walk.spawned;
        progress.changed += walk.changed;
        progress.bootstrapped = walk.finished;
        progress.bytes_read = batch.bytes_read();
        backlog.bootstrap_complete = walked?;
    }
    if sweep && let Some(state) = backlog.sweep.as_mut() {
        let mut walk = Walk::default();
        let walked = walk_roots(
            store,
            home,
            &mut batch,
            &mut state.after,
            recorded_at,
            |_, _| Ok(()),
            &mut walk,
        );
        progress.spawned += walk.spawned;
        progress.changed += walk.changed;
        progress.swept = walk.finished;
        progress.bytes_read = batch.bytes_read();
        walked?;
        if walk.complete {
            backlog.sweep = None;
        }
    }
    // Claude launches, in the watcher's passes only, after the spawn work,
    // within what is left of the same pass's bytes and time. What a failing
    // launch pass committed before its failure is still counted.
    if sweep && backlog.launches.pending() {
        let (bytes, time) = batch.remaining();
        let mut launches = LaunchProgress::default();
        let result = continue_claude_launches_into(
            store,
            home,
            &mut backlog.launches,
            &LaunchLimits::remaining(bytes, time),
            cancel,
            recorded_at,
            &mut launches,
        );
        progress.changed += launches.changed;
        progress.bytes_read += launches.bytes_read;
        progress.launches = Some(launches);
        result?;
    }
    Ok(())
}

/// How far one bootstrap call got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootstrapStep {
    /// Every recorded Codex locator has been read, now or by an earlier call.
    pub complete: bool,
    /// Root rollouts this call probed.
    pub probed: usize,
    /// Proved spawns this call handed to the store.
    pub spawned: usize,
    /// Proved spawns that changed what is stored.
    pub changed: usize,
    /// Locators this call finished, whether or not they were roots.
    pub finished: usize,
    pub bytes_read: u64,
}

/// Continue the one-time pass over the Codex sources the index already holds,
/// from where the last call stopped, until every recorded locator is read or
/// this call's budget, deadline or cancel stops it. Progress is saved after
/// each page, so a stopped pass resumes at the first locator it did not
/// finish; a finished pass does nothing. Only recorded root rollouts are
/// probed, each by its opening line.
pub fn bootstrap_codex_spawns(
    store: &mut Store,
    home: &Path,
    limits: TitleLimits,
    cancel: Option<&CancelToken>,
    recorded_at: i64,
) -> xt_store::Result<BootstrapStep> {
    let mut batch = Batch::new(limits, cancel);
    let mut walk = Walk::default();
    let complete = bootstrap_within(store, home, &mut batch, recorded_at, &mut walk)?;
    Ok(BootstrapStep {
        complete,
        probed: walk.probed,
        spawned: walk.spawned,
        changed: walk.changed,
        finished: walk.finished,
        bytes_read: batch.bytes_read(),
    })
}

/// Continue the bootstrap within `batch`, counting into `walk`; whether it
/// is complete.
fn bootstrap_within(
    store: &mut Store,
    home: &Path,
    batch: &mut Batch<'_>,
    recorded_at: i64,
    walk: &mut Walk,
) -> xt_store::Result<bool> {
    let kind = CreationEvidence::CodexThreadSpawn;
    let version = CODEX_THREAD_SPAWN_VERSION;
    let state = store.session_creation_bootstrap(kind, version)?;
    if state.complete {
        walk.complete = true;
        return Ok(true);
    }
    let mut after = state.after_locator;
    walk_roots(
        store,
        home,
        batch,
        &mut after,
        recorded_at,
        |store, progress| store.advance_session_creation_bootstrap(kind, version, progress),
        walk,
    )?;
    Ok(walk.complete)
}

/// How far one walk over the recorded Codex locators got.
#[derive(Default)]
struct Walk {
    complete: bool,
    probed: usize,
    spawned: usize,
    changed: usize,
    finished: usize,
}

/// Read the recorded Codex locators after `after` in key order, a page at a
/// time, probing each root rollout by its opening line and recording every
/// proved spawn, until none is left or the batch stops. `after` moves past
/// every locator finished, and `save` is told after each page (and once at
/// the end, as complete), so a stopped walk resumes at the first locator it
/// did not finish. What was recorded is counted into `walk` as each page
/// commits, so a walk that fails part-way still says what it changed.
fn walk_roots(
    store: &mut Store,
    home: &Path,
    batch: &mut Batch<'_>,
    after: &mut Option<String>,
    recorded_at: i64,
    mut save: impl FnMut(&mut Store, &CreationBootstrap) -> xt_store::Result<()>,
    walk: &mut Walk,
) -> xt_store::Result<()> {
    loop {
        if batch.stopped().is_some() {
            break;
        }
        let keys = store.source_locator_keys_after(
            SessionSource::ReadersCli,
            "codex:",
            after.as_deref(),
            BOOTSTRAP_PAGE,
        )?;
        if keys.is_empty() {
            walk.complete = true;
            save(
                store,
                &CreationBootstrap {
                    after_locator: after.clone(),
                    complete: true,
                },
            )?;
            break;
        }
        // Each key names the thread of its root rollout, or none.
        let roots: Vec<(String, Option<String>)> = keys
            .into_iter()
            .map(|key| {
                let native = root_native(&key);
                (key, native)
            })
            .collect();
        let natives: Vec<&str> = roots
            .iter()
            .filter_map(|(_, native)| native.as_deref())
            .collect();
        let probes = probe(store, home, &natives, batch)?;
        // Finished up to the first thread this call could not decide.
        let decided = &probes[..decided(&probes)];
        let halted = decided.len() < probes.len();
        let recorded = record(store, decided, recorded_at)?;
        walk.probed += decided.len();
        walk.spawned += recorded.dispositions.len();
        walk.changed += recorded.changed;
        // Every key before the first undecided root is finished; that root and
        // everything after it are read again by the next call.
        let mut remaining = decided.len();
        let mut finished = None;
        for (key, native) in &roots {
            if native.is_some() {
                if remaining == 0 {
                    break;
                }
                remaining -= 1;
            }
            finished = Some(key.clone());
            walk.finished += 1;
        }
        if let Some(key) = finished {
            save(
                store,
                &CreationBootstrap {
                    after_locator: Some(key.clone()),
                    complete: false,
                },
            )?;
            *after = Some(key);
        }
        if halted {
            break;
        }
    }
    Ok(())
}

/// The thread a Codex locator's file name makes it the root rollout of:
/// `codex:<dir>/rollout-<timestamp>-<thread>.jsonl`. A continuation, or any
/// other name, is none.
fn root_native(locator: &str) -> Option<String> {
    let path = Path::new(codex_path(locator)?);
    let stem = path
        .file_name()?
        .to_str()?
        .strip_prefix("rollout-")?
        .strip_suffix(".jsonl")?;
    let native = stem.get(stem.len().checked_sub(36)?..)?;
    (full_thread_id(native) && codex_segment(path, native) == Some(Segment::Root))
        .then(|| native.to_owned())
}

/// How a root rollout's locator for the thread `native` ends.
fn root_tail(native: &str) -> String {
    let tail = format!("{native}.jsonl");
    debug_assert_eq!(tail.chars().count(), LOCATOR_TAIL);
    tail
}

fn probe(
    store: &Store,
    home: &Path,
    natives: &[&str],
    batch: &mut Batch<'_>,
) -> xt_store::Result<Vec<SpawnProbe>> {
    let roots = host_roots(Host::Codex, home);
    let mut probes = Vec::with_capacity(natives.len());
    for native in natives {
        let mut probe = SpawnProbe {
            native_session_id: (*native).to_owned(),
            child_session_id: None,
            outcome: SpawnOutcome::NotSpawned,
        };
        probe.outcome = match batch.stopped() {
            Some(why) => SpawnOutcome::Refused(SpawnRefusal::Source(why)),
            None => probe_one(store, native, &roots, batch, &mut probe.child_session_id)?,
        };
        probes.push(probe);
    }
    // A cancel that landed at any point decides nothing that was read after
    // it; the whole call is left for later.
    if batch.cancelled() {
        for probe in &mut probes {
            probe.outcome = SpawnOutcome::Refused(SpawnRefusal::Source(Untitled::Cancelled));
        }
    }
    Ok(probes)
}

fn probe_one(
    store: &Store,
    native: &str,
    roots: &[std::path::PathBuf],
    batch: &mut Batch<'_>,
    child: &mut Option<String>,
) -> xt_store::Result<SpawnOutcome> {
    let refuse = |why| Ok(SpawnOutcome::Refused(why));
    if !full_thread_id(native) {
        return refuse(SpawnRefusal::InvalidIdentifier);
    }
    let sessions = store.user_sessions_with_native(Host::Codex, native)?;
    match sessions.as_slice() {
        [] => return refuse(SpawnRefusal::NotIndexed),
        [one] => *child = Some(one.clone()),
        _ => return refuse(SpawnRefusal::AmbiguousSession),
    }
    // Only the root rollout opens the thread; a continuation never says how
    // the thread was created, so it is not looked for. Every locator ending
    // in the root's name is found by one indexed seek, so a second recorded
    // root for this thread is seen too.
    let keys = store.source_locator_keys_ending(
        SessionSource::ReadersCli,
        &root_tail(native),
        MAX_CODEX_LOCATORS + 1,
    )?;
    if keys.len() > MAX_CODEX_LOCATORS {
        return refuse(SpawnRefusal::TooManySources);
    }
    let locators: Vec<IndexedSource> = keys
        .into_iter()
        .map(|locator| IndexedSource {
            locator,
            checkpoint: None,
        })
        .collect();
    let recorded: Vec<_> = locators
        .iter()
        .filter_map(|source| {
            let path = codex_path(&source.locator)?;
            (codex_segment(Path::new(path), native) == Some(Segment::Root))
                .then_some((path, source))
        })
        .collect();
    match recorded.len() {
        0 => return refuse(SpawnRefusal::NotIndexed),
        1 => {}
        _ => return refuse(SpawnRefusal::AmbiguousRoot),
    }
    let (mut candidates, outside) =
        match gather(recorded.into_iter(), native, roots, |path, roots| {
            codex_contained(path, roots, native)
        }) {
            Ok(found) => found,
            Err(why) => return refuse(SpawnRefusal::Source(why)),
        };
    if outside {
        return refuse(SpawnRefusal::Source(Untitled::Outside));
    }
    let Some(candidate) = candidates.pop().filter(|_| candidates.is_empty()) else {
        return refuse(SpawnRefusal::AmbiguousRoot);
    };
    Ok(
        match batch.opening_line(&candidate, &mut |line| spawn_header(line, native)) {
            Ok(outcome) => outcome,
            Err(why) => SpawnOutcome::Refused(SpawnRefusal::Source(why)),
        },
    )
}

/// Read a root rollout's opening line as the thread `native`'s creation.
///
/// It must be this thread's `session_meta`: `payload.id` is the thread. Its
/// `payload.source` is a spawn only as
/// `{"subagent": {"thread_spawn": {"parent_thread_id": <thread>}}}` with
/// nothing beside either key, and a parent that is a full thread identity
/// other than this one ([`typed_spawn`]). A spawned thread's header, as Codex
/// writes it, also names that parent in `payload.session_id`; this version
/// requires it present and exactly that parent, so a missing, null, child or
/// third thread there relates nothing. `session_id` only corroborates the
/// typed parent and never supplies one: in a header that is not a spawn it
/// must name the thread itself, when present. No other field —
/// `history_base`, depth, role, nickname or task path — is consulted.
fn spawn_header(line: &[u8], native: &str) -> SpawnOutcome {
    let refuse = SpawnOutcome::Refused;
    if !contains(line, b"session_meta") {
        return refuse(SpawnRefusal::NoOpeningHeader);
    }
    let Some(record) = object(line) else {
        return refuse(SpawnRefusal::NoOpeningHeader);
    };
    if record.get("type").and_then(Value::as_str) != Some("session_meta") {
        return refuse(SpawnRefusal::NoOpeningHeader);
    }
    let Some(payload) = record.get("payload").and_then(Value::as_object) else {
        return refuse(SpawnRefusal::IdentityMismatch);
    };
    if payload.get("id").and_then(Value::as_str) != Some(native) {
        return refuse(SpawnRefusal::IdentityMismatch);
    }
    if payload.get("source") == Some(&serde_json::json!({"subagent":{"other":"guardian"}})) {
        let Some(parent) = payload
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .filter(|parent| full_thread_id(parent) && *parent != native)
        else {
            return SpawnOutcome::NotSpawned;
        };
        return SpawnOutcome::Reviewer {
            parent_native_session_id: parent.to_owned(),
        };
    }
    match typed_spawn(payload, native) {
        TypedSpawn::None => {
            if payload
                .get("session_id")
                .is_some_and(|session| session.as_str() != Some(native))
            {
                refuse(SpawnRefusal::IdentityMismatch)
            } else {
                SpawnOutcome::NotSpawned
            }
        }
        TypedSpawn::Malformed => refuse(SpawnRefusal::MalformedSpawn),
        TypedSpawn::SelfSpawn => refuse(SpawnRefusal::SelfSpawn),
        TypedSpawn::Parent(parent) if !corroborated(payload, parent) => {
            refuse(SpawnRefusal::UncorroboratedSpawn)
        }
        TypedSpawn::Parent(parent) => SpawnOutcome::Spawned {
            parent_native_session_id: parent.to_owned(),
        },
    }
}

/// The store's disposition of each proved spawn, and how many changed what
/// is stored.
struct Recorded {
    dispositions: Vec<(String, CreationDisposition)>,
    changed: usize,
}

/// Hand every proved spawn to the store's guarded insertion.
fn record(
    store: &mut Store,
    probes: &[SpawnProbe],
    recorded_at: i64,
) -> xt_store::Result<Recorded> {
    let mut natives = Vec::new();
    let mut proofs = Vec::new();
    for probe in probes {
        if let (
            SpawnOutcome::Spawned {
                parent_native_session_id,
            },
            Some(child),
        ) = (&probe.outcome, &probe.child_session_id)
        {
            natives.push(probe.native_session_id.clone());
            proofs.push(SessionCreationProof {
                child_session_id: child.clone(),
                child_host: Host::Codex,
                child_native_session_id: probe.native_session_id.clone(),
                parent_host: Host::Codex,
                parent_native_session_id: parent_native_session_id.clone(),
                evidence_kind: CreationEvidence::CodexThreadSpawn,
                evidence_version: CODEX_THREAD_SPAWN_VERSION,
                witness: CreationWitness::RolloutOpeningSessionMeta,
            });
        }
    }
    let origins = probes
        .iter()
        .filter_map(|probe| {
            let SpawnOutcome::Reviewer {
                parent_native_session_id,
            } = &probe.outcome
            else {
                return None;
            };
            Some(xt_store::human_input::SessionOrigin {
                session_id: probe.child_session_id.clone()?,
                host: Host::Codex,
                native_session_id: probe.native_session_id.clone(),
                parent_host: Host::Codex,
                parent_native_session_id: parent_native_session_id.clone(),
                method: "native_reviewer_header".into(),
                evidence_id: "rollout_opening_session_meta".into(),
                launch_id: "native_reviewer_header".into(),
            })
        })
        .collect::<Vec<_>>();
    let origin_changes = if origins.is_empty() {
        0
    } else {
        let report =
            store.import_human_session_origins(&xt_store::human_input::OriginManifest {
                version: 1,
                sessions: origins,
            })?;
        report.applied + report.conflicted
    };
    if proofs.is_empty() {
        return Ok(Recorded {
            dispositions: Vec::new(),
            changed: origin_changes,
        });
    }
    let report = store.record_session_creations(&proofs, recorded_at)?;
    Ok(Recorded {
        dispositions: natives.into_iter().zip(report.dispositions).collect(),
        changed: report.changed + origin_changes,
    })
}
