//! The pull-request refresh: its lifecycle, its bounds and when it runs.
//!
//! A batch starts in one of two ways, and both only read GitHub through the
//! same bounded `gh pr view` client:
//!
//! * **Manual.** The user selects up to [`MAX_SELECTION`] pull requests and
//!   asks for them. A manual batch takes precedence: an automatic batch that
//!   holds the slot is cancelled and given a bounded time to stop.
//! * **Automatic.** [`PrRefreshService::auto_check`] picks the linked pull
//!   requests that were never checked, or were last seen open (merged and
//!   closed ones are final and never checked again automatically), and runs
//!   them through the same batch. The app calls it when its window gains
//!   focus, when the Dashboard is shown and about once an hour
//!   ([`AutoCheckWorker`]). A run that finished within [`AUTO_QUIET`] is not
//!   repeated unless a pull request appeared that was never checked, and an
//!   open pull request is checked again only after [`AUTO_RECHECK_AFTER`].
//!   A missing GitHub CLI, or one that answers with its documented
//!   "authentication required" exit, pauses automatic checks until the next
//!   launch or a manual refresh; it is never retried in a loop. A run whose
//!   first [`AUTO_GIVE_UP_AFTER`] checks all fail stops without pausing.
//!   Fixture mode's service is the injected synthetic one, so its automatic
//!   checks never resolve or run the GitHub CLI either.
//!
//! One batch at a time, and one pull request at a time inside it:
//!
//! * The selection is [`MAX_SELECTION`] stored pull-request IDs at most, each
//!   a positive ID that storage already holds; duplicates are refused. The
//!   IPC surface carries no URL, no repository and no executable path, so a
//!   caller cannot name a pull request, a host or a program that storage does
//!   not already know.
//! * The identities are resolved from storage under the state lock, which is
//!   then released before the first process starts. Storage is locked again
//!   only to persist one result, so every other command keeps working while a
//!   refresh is blocked on a process.
//! * The batch has a total budget ([`Budget::total`]) and each attempt its own
//!   ([`Budget::attempt`], and never more than the batch has left).
//! * A cancellation is honoured before the batch, between two attempts and
//!   during one. Pull requests the batch did not reach are reported as
//!   *skipped*, never as failed: nothing ran for them and nothing was written
//!   for them.
//! * Overlapping batches are refused rather than queued (except that a manual
//!   batch first cancels an automatic one), and after shutdown no batch can
//!   start at all.
//! * A result is persisted through the reviewed `Store::record_pr_refresh`,
//!   which owns the typed failure vocabulary, staleness and equal-attempt
//!   conflicts. This module decides none of those.
//! * The refresh event is emitted once, after the batch, and only when at
//!   least one attempt's result was actually committed.
use crate::{
    dto::{
        MAX_EXACT, PrAttemptOutcome, PrAutoCheckPause, PrAutoCheckStatus, PrList, PrPersistence,
        PrPersistenceError, PrRef, PrRefreshReport, PrRefreshRow, PrRefreshWriteReport,
        PrSkipReason,
    },
    gh_cli::{self, GhUnavailable},
    state::{AppState, StateError},
};
use std::{
    collections::HashSet,
    ffi::OsString,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use xt_probes::gh::{AttemptClock, CancelFlag, GhClient, GhClientError, Limits, SystemClock};
use xt_store::{
    Store,
    pr_link::{
        LinkedPullRequest, PrIdentity, PrRefreshError, PrState, RefreshOutcome, RefreshSuccess,
        RefreshWrite,
    },
};

/// Most pull requests one batch may name.
pub const MAX_SELECTION: usize = 20;
/// How long a whole batch may take.
pub const TOTAL_BUDGET: Duration = Duration::from_secs(120);
/// How long one attempt may take, never more than the batch has left.
pub const ATTEMPT_BUDGET: Duration = Duration::from_secs(30);
/// Less budget than this left is not an attempt: a slice too short to answer
/// in would only manufacture a timeout, so the pull request is skipped.
pub const MIN_ATTEMPT: Duration = Duration::from_secs(1);
/// How long shutdown waits for a cancelled batch to stop before it gives up
/// waiting. Storage refuses writes once it is closed either way.
pub const SHUTDOWN_BOUND: Duration = Duration::from_secs(6);
/// An open pull request is checked again automatically once its last attempt
/// is at least this old.
pub const AUTO_RECHECK_AFTER: Duration = Duration::from_secs(60 * 60);
/// After an automatic run, another one waits at least this long, unless a
/// pull request that was never checked appeared after a run that went cleanly.
pub const AUTO_QUIET: Duration = Duration::from_secs(10 * 60);
/// How often the app wakes the automatic check when nothing else does.
pub const AUTO_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Most batches one automatic run makes; the rest wait for the next run.
pub const AUTO_MAX_BATCHES: usize = 5;
/// An automatic run whose first this-many attempts all fail, before any
/// succeeds, stops: a cost cap, so an offline host or a broken gh costs a few
/// requests rather than a run of them. It does not pause automatic checks;
/// the next run tries again.
pub const AUTO_GIVE_UP_AFTER: usize = 3;

/// The bounds of one batch. Tests shorten them; the defaults are the
/// documented ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub total: Duration,
    pub attempt: Duration,
    pub minimum: Duration,
    pub shutdown: Duration,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            total: TOTAL_BUDGET,
            attempt: ATTEMPT_BUDGET,
            minimum: MIN_ATTEMPT,
            shutdown: SHUTDOWN_BOUND,
        }
    }
}

/// Called after a batch that committed at least one result (the app emits the
/// refresh event to the frontend).
pub type Publish = Arc<dyn Fn() + Send + Sync>;

/// Runs once after the selection has been resolved and before the first
/// attempt, with the batch's cancel flag. Production installs none; a test
/// uses it to arrange a cancellation that arrives before anything has run.
pub type BeforeAttempts = Arc<dyn Fn(&CancelFlag) + Send + Sync>;

/// One bounded attempt for one identity.
pub struct AttemptRequest<'a> {
    pub identity: &'a PrIdentity,
    /// The slice of the batch's budget this attempt may use.
    pub deadline: Duration,
    /// Cancelled from another thread; the attempt must stop within its bound.
    pub cancel: &'a CancelFlag,
    /// Read once per attempt, and it stamps the stored result.
    pub clock: &'a dyn AttemptClock,
}

/// What actually performs an attempt. Production runs the reviewed bounded
/// `gh pr view` client; fixture mode and tests inject synthetic outcomes, or
/// run the same client over a synthetic executable.
pub type Attempt =
    Arc<dyn Fn(&AttemptRequest<'_>) -> Result<RefreshOutcome, GhClientError> + Send + Sync>;

enum Source {
    /// The GitHub CLI, resolved when the user asks and never before.
    GitHubCli {
        explicit: Option<OsString>,
    },
    Injected(Attempt),
}

/// Why a batch could not start, or could not start attempting. None of these
/// writes anything.
#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error("select at least one pull request to refresh")]
    EmptySelection,
    #[error("select at most {MAX_SELECTION} pull requests to refresh at a time")]
    SelectionTooLarge,
    #[error("a pull request refresh accepts stored pull request identifiers only")]
    InvalidSelection,
    #[error("a pull request was selected twice")]
    DuplicateSelection,
    #[error("a pull request refresh is already running")]
    AlreadyRunning,
    #[error("the application is shutting down")]
    Stopped,
    #[error("{0}")]
    Unavailable(#[from] GhUnavailable),
    #[error(transparent)]
    State(#[from] StateError),
}

/// The storage one batch resolves its selection against and persists into.
/// The application state is one; the fixture exporter's own store is another.
pub struct RefreshStorage<'a> {
    /// Resolve the selection under whatever lock storage needs, and release
    /// it: nothing holds it while an attempt runs.
    pub targets: &'a dyn Fn() -> Result<Vec<PrTarget>, StateError>,
    /// Persist one executed attempt's result.
    pub record: &'a dyn Fn(&RefreshOutcome) -> Result<RefreshWrite, StateError>,
}

/// One selected ID resolved against storage.
pub enum PrTarget {
    Ready {
        reference: PrRef,
        identity: PrIdentity,
    },
    Skipped {
        id: i64,
        reference: Option<PrRef>,
        reason: PrSkipReason,
    },
}

impl PrTarget {
    pub fn skipped(id: i64, reference: Option<PrRef>, reason: PrSkipReason) -> Self {
        Self::Skipped {
            id,
            reference,
            reason,
        }
    }
}

/// The one batch that may be running, and whether it was started
/// automatically (a manual batch may cancel only that kind).
struct Active {
    cancel: CancelFlag,
    automatic: bool,
}

/// What the automatic check remembers between runs, for this launch only.
#[derive(Default)]
struct AutoState {
    disabled: bool,
    running: bool,
    paused: Option<PrAutoCheckPause>,
    /// Refresh-clock milliseconds when the last run that started a batch ended.
    last_finished_at: Option<i64>,
    /// That run attempted everything it selected, without a stop or skip.
    last_clean: bool,
}

/// What one [`PrRefreshService::auto_check`] call did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutoRun {
    /// Switched off for this launch; nothing is read or run.
    Disabled,
    /// The application is shutting down.
    Stopped,
    /// Paused until a manual refresh or the next launch; nothing ran.
    Paused(PrAutoCheckPause),
    /// Another batch, manual or automatic, holds the slot; nothing ran.
    Busy,
    /// Storage could not be read; nothing ran.
    Unreadable,
    /// No linked pull request is due for a check.
    NothingDue,
    /// A run finished within [`AUTO_QUIET`] and nothing new appeared since.
    Quiet,
    /// At least one batch started.
    Ran(AutoReport),
}

/// The batches one automatic run made.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutoReport {
    pub batches: u32,
    /// Selected pull requests over every batch.
    pub requested: u32,
    pub attempted: u32,
    pub committed: bool,
    /// Every selected pull request was attempted and nothing stopped a batch.
    pub clean: bool,
    /// Set when this run paused automatic checks.
    pub paused: Option<PrAutoCheckPause>,
}

/// The pull requests an automatic run should check next.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutoSelection {
    /// At most [`MAX_SELECTION`] stored IDs: never checked first (newest
    /// stored first), then open ones whose last check is oldest, then ones
    /// whose last check failed, oldest first.
    pub ids: Vec<i64>,
    /// How many due pull requests were never checked at all.
    pub never_checked: usize,
}

/// Choose what an automatic run checks, from one storage snapshot.
///
/// Only pull requests a session still links are considered. Merged and closed
/// ones are final and never chosen. One with no attempt at all is due now;
/// one whose last success said open, or whose last attempt failed, is due
/// once its last attempt is [`AUTO_RECHECK_AFTER`] old. Ones whose last
/// attempt failed come last, so a few pull requests gh cannot open (another
/// account's repository, a deleted one) never crowd out the rest. `tried` excludes the
/// IDs this run already attempted, so a result storage did not apply cannot
/// make a run pick the same pull request twice.
pub fn auto_selection(
    snapshot: &[LinkedPullRequest],
    now_ms: i64,
    tried: &HashSet<i64>,
) -> AutoSelection {
    let recheck = i64::try_from(AUTO_RECHECK_AFTER.as_millis()).unwrap_or(i64::MAX);
    let mut never = Vec::new();
    let mut due = Vec::new();
    let mut failed = Vec::new();
    for row in snapshot.iter().filter(|row| row.linked_sessions > 0) {
        let stored = &row.pull_request;
        if tried.contains(&stored.id) || !(1..=MAX_EXACT).contains(&stored.id) {
            continue;
        }
        if matches!(stored.state, Some(PrState::Merged | PrState::Closed)) {
            continue;
        }
        match stored.last_attempted_at {
            None => never.push(stored.id),
            Some(at) if now_ms.saturating_sub(at) >= recheck => {
                if stored.refresh_error.is_some() {
                    failed.push((at, stored.id));
                } else {
                    due.push((at, stored.id));
                }
            }
            Some(_) => {}
        }
    }
    never.sort_unstable_by(|a, b| b.cmp(a));
    due.sort_unstable_by(|(a_at, a_id), (b_at, b_id)| a_at.cmp(b_at).then(b_id.cmp(a_id)));
    failed.sort_unstable_by(|(a_at, a_id), (b_at, b_id)| a_at.cmp(b_at).then(b_id.cmp(a_id)));
    let never_checked = never.len();
    let ids = never
        .into_iter()
        .chain(due.into_iter().map(|(_, id)| id))
        .chain(failed.into_iter().map(|(_, id)| id))
        .take(MAX_SELECTION)
        .collect();
    AutoSelection { ids, never_checked }
}

pub struct PrRefreshService {
    source: Source,
    clock: Arc<dyn AttemptClock + Send + Sync>,
    budget: Budget,
    publish: Publish,
    /// Called whenever the automatic check's status changes.
    auto_publish: Publish,
    before_attempts: Option<BeforeAttempts>,
    /// The one batch that may be running.
    active: Mutex<Option<Active>>,
    idle: Condvar,
    stopped: AtomicBool,
    /// A manual batch is waiting for an automatic one to give way; no new
    /// automatic batch starts meanwhile.
    manual_waiting: AtomicBool,
    auto: Mutex<AutoState>,
}

impl PrRefreshService {
    /// The production service: it resolves the GitHub CLI only when a refresh
    /// is actually requested, so an app that is never asked never looks for it.
    pub fn production(explicit: Option<OsString>, publish: Publish) -> Self {
        Self::with_source(Source::GitHubCli { explicit }, publish)
    }

    /// A service whose attempts are supplied. Fixture mode and tests use this;
    /// it resolves no executable and launches no process of its own.
    pub fn injected(attempt: Attempt, publish: Publish) -> Self {
        Self::with_source(Source::Injected(attempt), publish)
    }

    /// Fixture mode: deterministic synthetic outcomes over the disposable
    /// fixture database, stamped with the fixture's pinned instant. The GitHub
    /// CLI is never resolved, no process runs, no credential is read, no
    /// network request is made and no real pull request is contacted.
    pub fn fixture(now_ms: i64, publish: Publish) -> Self {
        Self::injected(
            Arc::new(|request: &AttemptRequest<'_>| {
                Ok(fixture_outcome(
                    request.identity,
                    request.clock.attempted_at(),
                ))
            }),
            publish,
        )
        .with_clock(Arc::new(PinnedClock(now_ms)))
    }

    fn with_source(source: Source, publish: Publish) -> Self {
        Self {
            source,
            clock: Arc::new(SystemClock),
            budget: Budget::default(),
            publish,
            auto_publish: Arc::new(|| {}),
            before_attempts: None,
            active: Mutex::new(None),
            idle: Condvar::new(),
            stopped: AtomicBool::new(false),
            manual_waiting: AtomicBool::new(false),
            auto: Mutex::new(AutoState::default()),
        }
    }

    /// Called whenever the automatic check's status changes (the app emits
    /// its status event to the frontend).
    pub fn with_auto_publish(mut self, publish: Publish) -> Self {
        self.auto_publish = publish;
        self
    }

    /// Switch the automatic check off for this service's lifetime: every
    /// [`Self::auto_check`] then reads and runs nothing.
    pub fn without_auto_check(self) -> Self {
        lock(&self.auto).disabled = true;
        self
    }

    /// Shorter bounds, for tests.
    pub fn with_budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    /// A clock other than the host's, for tests that order attempts.
    pub fn with_clock(mut self, clock: Arc<dyn AttemptClock + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    /// See [`BeforeAttempts`].
    pub fn with_before_attempts(mut self, hook: BeforeAttempts) -> Self {
        self.before_attempts = Some(hook);
        self
    }

    pub fn budget(&self) -> Budget {
        self.budget
    }

    /// Whether a batch is running now.
    pub fn is_active(&self) -> bool {
        lock(&self.active).is_some()
    }

    /// Ask the running batch to stop. Returns false when none was running:
    /// a cancellation is never remembered for a later batch.
    pub fn cancel(&self) -> bool {
        match lock(&self.active).as_ref() {
            Some(active) => {
                active.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Whether [`Self::shutdown`] has been called.
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Where the automatic check stands now.
    pub fn auto_status(&self) -> PrAutoCheckStatus {
        let auto = lock(&self.auto);
        PrAutoCheckStatus {
            enabled: !auto.disabled,
            checking: auto.running,
            paused: auto.paused,
            last_finished_at_ms: auto
                .last_finished_at
                .filter(|at| (0..=MAX_EXACT).contains(at)),
        }
    }

    /// Refuse every later batch, cancel the running one and wait a bounded
    /// time for it to stop, so the batch has released storage before the
    /// application closes it. A batch that outlives the wait still cannot
    /// write: storage refuses a closed database, and reports it as skipped.
    pub fn shutdown(&self) -> bool {
        self.stopped.store(true, Ordering::SeqCst);
        let mut active = lock(&self.active);
        if let Some(active) = active.as_ref() {
            active.cancel.cancel();
        }
        let deadline = Instant::now() + self.budget.shutdown;
        while active.is_some() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (guard, timeout) = self
                .idle
                .wait_timeout(active, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            active = guard;
            if timeout.timed_out() && active.is_some() {
                return false;
            }
        }
        true
    }

    /// Refresh a selection of stored pull requests, sequentially, within the
    /// batch's budget. The selection's order is the report's order.
    ///
    /// This is the manual refresh: an automatic batch that is running gives
    /// way to it, and automatic checks that paused may try again afterwards.
    pub fn refresh(&self, state: &AppState, ids: &[i64]) -> Result<PrRefreshReport, RefreshError> {
        validate_selection(ids)?;
        self.resume_auto();
        self.refresh_into(RefreshStorage {
            targets: &|| state.pr_refresh_targets(ids),
            record: &|outcome| state.record_pr_refresh(outcome),
        })
    }

    /// One batch against the storage the caller names. The application
    /// command resolves and persists through its own state; the fixture
    /// exporter uses the store it owns, so both are accounted for here and
    /// there is only one implementation of what a batch did.
    pub fn refresh_into(
        &self,
        storage: RefreshStorage<'_>,
    ) -> Result<PrRefreshReport, RefreshError> {
        self.batch(storage, None)
    }

    /// One batch. `automatic` is the state of an automatic run: such a batch
    /// never displaces another one, and answers that say the GitHub CLI
    /// cannot be used, or is rate limited, stop it early.
    fn batch(
        &self,
        storage: RefreshStorage<'_>,
        automatic: Option<&RunTrack>,
    ) -> Result<PrRefreshReport, RefreshError> {
        // The guard refuses an overlapping batch and, whatever ends this call
        // (a return, an error or a panic), releases the active slot again.
        let guard = self.begin(automatic.is_some())?;
        // Nothing has been read or run yet: an unresolvable executable ends
        // the batch before storage is touched.
        let attempt = self.attempt()?;
        let attempt = match automatic {
            Some(run) => automatic_attempt(attempt, Arc::clone(run)),
            None => attempt,
        };
        let targets = (storage.targets)()?;
        if let Some(hook) = &self.before_attempts {
            hook(&guard.cancel);
        }
        let started = Instant::now();
        let mut rows: Vec<PrRefreshRow> = Vec::with_capacity(targets.len());
        // Once a condition stops the batch, every pull request after it is
        // skipped for the same reason rather than attempted.
        let mut stopped: Option<PrSkipReason> = None;
        for target in targets {
            let (reference, identity) = match target {
                PrTarget::Skipped {
                    id,
                    reference,
                    reason,
                } => {
                    rows.push(skipped_row(id, reference, reason));
                    continue;
                }
                PrTarget::Ready {
                    reference,
                    identity,
                } => (reference, identity),
            };
            if let Some(reason) = stopped {
                rows.push(skipped_row(reference.id, Some(reference), reason));
                continue;
            }
            if guard.cancel.is_cancelled() {
                stopped = Some(PrSkipReason::Cancelled);
                rows.push(skipped_row(
                    reference.id,
                    Some(reference),
                    PrSkipReason::Cancelled,
                ));
                continue;
            }
            let remaining = self.budget.total.saturating_sub(started.elapsed());
            if remaining < self.budget.minimum {
                stopped = Some(PrSkipReason::BudgetExhausted);
                rows.push(skipped_row(
                    reference.id,
                    Some(reference),
                    PrSkipReason::BudgetExhausted,
                ));
                continue;
            }
            let outcome = attempt(&AttemptRequest {
                identity: &identity,
                deadline: self.budget.attempt.min(remaining),
                cancel: &guard.cancel,
                clock: &*self.clock,
            });
            let Ok(outcome) = outcome else {
                // The attempt could not be started at all. The condition is
                // the host's, not this pull request's, so it ends the batch,
                // and nothing was executed for this pull request either.
                stopped = Some(PrSkipReason::AttemptRefused);
                rows.push(skipped_row(
                    reference.id,
                    Some(reference),
                    PrSkipReason::AttemptRefused,
                ));
                continue;
            };
            // The attempt executed. What storage then does with its result is
            // reported beside it and never turns it back into a non-event.
            let persistence = match (storage.record)(&outcome) {
                Ok(write) => PrPersistence::Recorded {
                    write: write.into(),
                },
                Err(error) => {
                    let reason = persistence_error(&error);
                    // Storage that closed or failed stays that way, so nothing
                    // after this is attempted and no further request is made;
                    // a result it refused for this one identity does not stop
                    // the batch.
                    if !matches!(reason, PrPersistenceError::Refused) {
                        stopped = Some(PrSkipReason::StorageUnavailable);
                    }
                    PrPersistence::NotRecorded { reason }
                }
            };
            rows.push(PrRefreshRow {
                id: reference.id,
                pull_request: Some(reference),
                outcome: executed(&outcome, persistence),
            });
        }
        let report = report(rows, guard.cancel.is_cancelled());
        if report.committed {
            (self.publish)();
        }
        Ok(report)
    }

    /// Resolve what will perform this batch's attempts. The production path
    /// resolves the executable exactly here: once per requested batch, so a
    /// GitHub CLI installed later is found without restarting the app.
    fn attempt(&self) -> Result<Attempt, RefreshError> {
        match &self.source {
            Source::Injected(attempt) => Ok(Arc::clone(attempt)),
            Source::GitHubCli { explicit } => {
                let executable = gh_cli::resolve_from_environment(explicit.as_deref())?;
                Ok(Arc::new(move |request: &AttemptRequest<'_>| {
                    // Only the deadline changes per attempt; the response
                    // bounds stay the reviewed client's own.
                    let client = GhClient::with_limits(
                        &executable,
                        Limits {
                            deadline: request.deadline,
                            ..Limits::default()
                        },
                    )?;
                    client.pr_view(request.identity, request.clock, Some(request.cancel))
                }))
            }
        }
    }

    fn begin(&self, automatic: bool) -> Result<ActiveGuard<'_>, RefreshError> {
        let mut active = lock(&self.active);
        if self.stopped.load(Ordering::SeqCst) {
            return Err(RefreshError::Stopped);
        }
        if automatic && self.manual_waiting.load(Ordering::SeqCst) {
            return Err(RefreshError::AlreadyRunning);
        }
        if !automatic && active.as_ref().is_some_and(|running| running.automatic) {
            // A person asked: the automatic batch is cancelled and given the
            // shutdown bound to release the slot. No new automatic batch can
            // start while this waits.
            self.manual_waiting.store(true, Ordering::SeqCst);
            let deadline = Instant::now() + self.budget.shutdown;
            while let Some(running) = active.as_ref().filter(|running| running.automatic) {
                running.cancel.cancel();
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                active = self
                    .idle
                    .wait_timeout(active, remaining)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
            }
            self.manual_waiting.store(false, Ordering::SeqCst);
            if self.stopped.load(Ordering::SeqCst) {
                return Err(RefreshError::Stopped);
            }
        }
        if active.is_some() {
            return Err(RefreshError::AlreadyRunning);
        }
        let cancel = CancelFlag::new();
        *active = Some(Active {
            cancel: cancel.clone(),
            automatic,
        });
        Ok(ActiveGuard {
            service: self,
            cancel,
        })
    }

    /// A manual refresh lets automatic checks that paused try again.
    fn resume_auto(&self) {
        let resumed = lock(&self.auto).paused.take().is_some();
        if resumed {
            (self.auto_publish)();
        }
    }

    /// Check, without being asked, the linked pull requests that were never
    /// checked or were last seen open, in batches of at most
    /// [`MAX_SELECTION`], through the same bounded batch as a manual refresh.
    ///
    /// Cheap to call often: it reads storage only when it is not disabled,
    /// paused or busy, and starts a batch only when something is due and the
    /// last run did not finish within [`AUTO_QUIET`] (a pull request never
    /// checked skips that wait when the last run went cleanly). A run stops at
    /// [`AUTO_MAX_BATCHES`] or at the first batch that did not attempt
    /// everything it selected, or after [`AUTO_GIVE_UP_AFTER`] failures before
    /// any success. A GitHub CLI that is missing or not signed in pauses it
    /// until [`Self::refresh`] or the next launch.
    pub fn auto_check(&self, state: &AppState) -> AutoRun {
        let (last_finished_at, last_clean) = {
            let mut auto = lock(&self.auto);
            if auto.disabled {
                return AutoRun::Disabled;
            }
            if self.is_stopped() {
                return AutoRun::Stopped;
            }
            if let Some(reason) = auto.paused {
                return AutoRun::Paused(reason);
            }
            if auto.running || self.is_active() {
                return AutoRun::Busy;
            }
            auto.running = true;
            (auto.last_finished_at, auto.last_clean)
        };
        // Whatever ends this call, the run is no longer running afterwards.
        let mut running = AutoRunning {
            service: self,
            announced: false,
        };
        let mut tried = HashSet::new();
        let now = self.clock.attempted_at();
        let Ok(mut selection) = state.pr_auto_selection(now, &tried) else {
            return AutoRun::Unreadable;
        };
        if selection.ids.is_empty() {
            return AutoRun::NothingDue;
        }
        let quiet = i64::try_from(AUTO_QUIET.as_millis()).unwrap_or(i64::MAX);
        let recent = last_finished_at.is_some_and(|at| now.saturating_sub(at) < quiet);
        if recent && !(last_clean && selection.never_checked > 0) {
            return AutoRun::Quiet;
        }
        running.announced = true;
        (self.auto_publish)();
        let mut report = AutoReport {
            clean: true,
            ..AutoReport::default()
        };
        let run: RunTrack = Arc::new(Mutex::new(RunState::default()));
        loop {
            tried.extend(selection.ids.iter().copied());
            let ids = std::mem::take(&mut selection.ids);
            let outcome = self.batch(
                RefreshStorage {
                    targets: &|| state.pr_refresh_targets(&ids),
                    record: &|outcome| state.record_pr_refresh(outcome),
                },
                Some(&run),
            );
            report.batches += 1;
            match outcome {
                Ok(batch) => {
                    report.requested += batch.requested;
                    report.attempted += batch.attempted;
                    report.committed |= batch.committed;
                    if batch.cancelled || batch.skipped > 0 {
                        report.clean = false;
                    }
                }
                Err(RefreshError::Unavailable(_)) => {
                    lock(&run).pause.get_or_insert(PrAutoCheckPause::GhMissing);
                    report.clean = false;
                }
                Err(_) => report.clean = false,
            }
            report.paused = lock(&run).pause;
            if report.paused.is_some()
                || !report.clean
                || usize::try_from(report.batches).is_ok_and(|n| n >= AUTO_MAX_BATCHES)
            {
                break;
            }
            match state.pr_auto_selection(self.clock.attempted_at(), &tried) {
                Ok(next) if !next.ids.is_empty() => selection = next,
                _ => break,
            }
        }
        {
            let mut auto = lock(&self.auto);
            auto.last_finished_at = Some(self.clock.attempted_at());
            auto.last_clean = report.clean;
            if report.paused.is_some() {
                auto.paused = report.paused;
            }
        }
        drop(running);
        AutoRun::Ran(report)
    }

    fn finish(&self) {
        let mut active = lock(&self.active);
        *active = None;
        drop(active);
        self.idle.notify_all();
    }
}

/// The fixture clock: every attempt in a fixture is stamped with the
/// fixture's own pinned instant, so an export is the same on every host and
/// at any time of day.
struct PinnedClock(i64);

impl AttemptClock for PinnedClock {
    fn attempted_at(&self) -> i64 {
        self.0
    }
}

/// What a fixture pull request answers. The pull-request number picks the
/// answer, so a fixture shows an open, a merged and a failed refresh without
/// declaring any of them, and the same identity always answers the same way.
fn fixture_outcome(identity: &PrIdentity, attempted_at: i64) -> RefreshOutcome {
    let number = identity.number();
    if number.is_multiple_of(3) {
        return RefreshOutcome::Failure(xt_store::pr_link::RefreshFailure {
            pull_request: identity.clone(),
            attempted_at,
            error: xt_store::pr_link::PrRefreshError::RateLimited,
        });
    }
    let merged = number % 3 == 2;
    RefreshOutcome::Success(RefreshSuccess {
        pull_request: identity.clone(),
        attempted_at,
        title: format!("Synthetic fixture pull request {number}"),
        state: if merged {
            PrState::Merged
        } else {
            PrState::Open
        },
        merged_at: merged.then(|| {
            jiff::Timestamp::from_millisecond(attempted_at)
                .map(|instant| instant.to_string())
                .unwrap_or_else(|_| "2026-01-01T00:00:00Z".to_owned())
        }),
        additions: i64::try_from(number).unwrap_or(i64::MAX) * 3,
        deletions: i64::try_from(number).unwrap_or(i64::MAX),
        head_ref_name: format!("fixture/pull-{number}"),
    })
}

/// The deterministic pull-request reports a fixture export carries: the
/// stored list, one synthetic refresh of every listed pull request at the
/// fixture's pinned instant, and the list that refresh leaves behind.
///
/// It runs through the same batch as the application command, so the exported
/// report is produced by the accounting the product uses, not by a second
/// description of it.
pub fn fixture_reports(
    store: &mut Store,
    now_ms: i64,
) -> Result<FixturePullRequests, RefreshError> {
    let snapshot = store.linked_pull_requests().map_err(StateError::from)?;
    let listed = crate::state::pr_list(snapshot.clone()).map_err(StateError::PrEncoding)?;
    let ids: Vec<i64> = listed.rows.iter().map(|row| row.pull_request.id).collect();
    // A fixture that links no pull request has nothing to refresh: it exports
    // empty lists and a report that attempted nothing. Nothing runs, so the
    // user command's rule that a selection is never empty is not in play here;
    // `refresh` still enforces it for every request a user makes.
    if ids.is_empty() {
        return Ok(FixturePullRequests {
            refreshed: listed.clone(),
            listed,
            refresh: report(Vec::new(), false),
        });
    }
    validate_selection(&ids)?;
    let store = std::cell::RefCell::new(store);
    let refresh =
        PrRefreshService::fixture(now_ms, Arc::new(|| {})).refresh_into(RefreshStorage {
            targets: &|| {
                crate::state::pr_refresh_targets(&snapshot, &ids).map_err(StateError::PrEncoding)
            },
            record: &|outcome| Ok(store.borrow_mut().record_pr_refresh(outcome)?),
        })?;
    let after = store
        .borrow()
        .linked_pull_requests()
        .map_err(StateError::from)?;
    Ok(FixturePullRequests {
        listed,
        refresh,
        refreshed: crate::state::pr_list(after).map_err(StateError::PrEncoding)?,
    })
}

/// What [`fixture_reports`] produced, in the order a consumer sees it.
pub struct FixturePullRequests {
    pub listed: PrList,
    pub refresh: PrRefreshReport,
    pub refreshed: PrList,
}

/// What one automatic run has seen so far, across its batches.
#[derive(Default)]
struct RunState {
    /// The pause the run's answers asked for, if any.
    pause: Option<PrAutoCheckPause>,
    /// At least one attempt in this run succeeded.
    succeeded: bool,
    /// Failed attempts since the run started, while none has succeeded.
    failures: usize,
}

type RunTrack = Arc<Mutex<RunState>>;

/// Wrap an automatic batch's attempts:
///
/// * An answer that says the GitHub CLI cannot be used — not signed in (gh's
///   documented exit 4), or not runnable — records a pause, and that or a
///   rate limit cancels the rest of the batch. An attempt that could not
///   start at all already ends the batch; it pauses as well. Only these
///   pause: other failures say nothing reliable about the GitHub CLI.
/// * Before anything in the run has succeeded, [`AUTO_GIVE_UP_AFTER`] other
///   failures stop the run without pausing: a cost cap for an offline host
///   or a broken gh. Once one attempt has succeeded, a failure is that pull
///   request's own (deleted, another account's repository) and never stops
///   the others.
/// * A failure the run's own cancellation caused counts for nothing.
fn automatic_attempt(inner: Attempt, run: RunTrack) -> Attempt {
    Arc::new(move |request: &AttemptRequest<'_>| {
        let outcome = inner(request);
        let mut run = lock(&run);
        let (reason, stop) = match &outcome {
            Err(_) => (Some(PrAutoCheckPause::GhMissing), false),
            Ok(RefreshOutcome::Failure(failure)) => match failure.error {
                PrRefreshError::Unauthorized => (Some(PrAutoCheckPause::GhSignedOut), true),
                PrRefreshError::Unavailable => (Some(PrAutoCheckPause::GhMissing), true),
                PrRefreshError::RateLimited => (None, true),
                PrRefreshError::Cancelled => (None, false),
                _ => {
                    if !run.succeeded {
                        run.failures += 1;
                    }
                    (None, !run.succeeded && run.failures >= AUTO_GIVE_UP_AFTER)
                }
            },
            Ok(RefreshOutcome::Success(_)) => {
                run.succeeded = true;
                (None, false)
            }
        };
        if let Some(reason) = reason {
            run.pause.get_or_insert(reason);
        }
        if stop {
            request.cancel.cancel();
        }
        outcome
    })
}

/// Marks an automatic run finished on every outcome, including a panic, and
/// announces the change when the run had announced its start.
struct AutoRunning<'a> {
    service: &'a PrRefreshService,
    announced: bool,
}

impl Drop for AutoRunning<'_> {
    fn drop(&mut self) {
        lock(&self.service.auto).running = false;
        if self.announced {
            (self.service.auto_publish)();
        }
    }
}

/// Wakes the automatic check from one background thread, so runs never
/// overlap each other: on request (window focus, the Dashboard shown) and
/// otherwise every `interval`. Requests that arrive while a run is going are
/// folded into one more pass. The thread ends when `run` returns false or the
/// worker is stopped.
pub struct AutoCheckWorker {
    wake: Mutex<Option<mpsc::Sender<()>>>,
}

impl AutoCheckWorker {
    pub fn start(interval: Duration, run: impl Fn() -> bool + Send + 'static) -> Self {
        let (wake, woken) = mpsc::channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("pr-auto-check".into())
            .spawn(move || {
                loop {
                    match woken.recv_timeout(interval) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                    while woken.try_recv().is_ok() {}
                    if !run() {
                        return;
                    }
                }
            });
        Self {
            wake: Mutex::new(spawned.is_ok().then_some(wake)),
        }
    }

    /// A worker that never runs anything.
    pub fn disabled() -> Self {
        Self {
            wake: Mutex::new(None),
        }
    }

    /// Ask for a run soon. Never blocks; a stopped worker ignores it.
    pub fn wake(&self) {
        if let Some(wake) = lock(&self.wake).as_ref() {
            let _ = wake.send(());
        }
    }

    /// End the thread after any run in progress.
    pub fn stop(&self) {
        lock(&self.wake).take();
    }
}

/// Holds the one active slot for the length of a batch and releases it on
/// every outcome, including a panic inside the batch.
struct ActiveGuard<'a> {
    service: &'a PrRefreshService,
    cancel: CancelFlag,
}

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.service.finish();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn validate_selection(ids: &[i64]) -> Result<(), RefreshError> {
    if ids.is_empty() {
        return Err(RefreshError::EmptySelection);
    }
    if ids.len() > MAX_SELECTION {
        return Err(RefreshError::SelectionTooLarge);
    }
    if ids.iter().any(|id| !(1..=MAX_EXACT).contains(id)) {
        return Err(RefreshError::InvalidSelection);
    }
    let mut seen = HashSet::with_capacity(ids.len());
    if !ids.iter().all(|id| seen.insert(*id)) {
        return Err(RefreshError::DuplicateSelection);
    }
    Ok(())
}

fn skipped_row(id: i64, reference: Option<PrRef>, reason: PrSkipReason) -> PrRefreshRow {
    PrRefreshRow {
        id,
        pull_request: reference,
        outcome: PrAttemptOutcome::Skipped { reason },
    }
}

/// Why storage did not keep one executed attempt's result.
///
/// Only the store's own validation vocabulary (`InvalidInput`: an equal-attempt
/// conflict, an identity it no longer holds, a result its rules reject) is a
/// refusal of *this* result, after which the batch goes on. Everything else —
/// an SQLite I/O, corruption, full-disk or busy failure, undecodable stored
/// data, an incompatible schema, a poisoned lock — says storage itself cannot
/// be relied on, so it is unusable and the batch must stop making requests.
fn persistence_error(error: &StateError) -> PrPersistenceError {
    match error {
        StateError::Closed => PrPersistenceError::StorageClosed,
        StateError::Store(xt_store::Error::InvalidInput(_)) => PrPersistenceError::Refused,
        _ => PrPersistenceError::Unusable,
    }
}

/// One executed attempt: its own typed outcome, and separately what storage
/// did with the result.
fn executed(outcome: &RefreshOutcome, persistence: PrPersistence) -> PrAttemptOutcome {
    match outcome {
        RefreshOutcome::Success(_) => PrAttemptOutcome::Succeeded { persistence },
        RefreshOutcome::Failure(failure) => PrAttemptOutcome::Failed {
            error: failure.error.into(),
            persistence,
        },
    }
}

/// Every counter is derived from the rows, so the report cannot disagree with
/// what the batch actually did. Execution and persistence are counted
/// separately: `attempted` counts what ran, `unrecorded` counts how much of
/// that storage did not keep.
fn report(rows: Vec<PrRefreshRow>, cancelled: bool) -> PrRefreshReport {
    let count = |predicate: fn(&PrAttemptOutcome) -> bool| {
        u32::try_from(rows.iter().filter(|row| predicate(&row.outcome)).count()).unwrap_or(u32::MAX)
    };
    let succeeded = count(|outcome| matches!(outcome, PrAttemptOutcome::Succeeded { .. }));
    let failed = count(|outcome| matches!(outcome, PrAttemptOutcome::Failed { .. }));
    let skipped = count(|outcome| matches!(outcome, PrAttemptOutcome::Skipped { .. }));
    PrRefreshReport {
        requested: succeeded + failed + skipped,
        attempted: succeeded + failed,
        succeeded,
        failed,
        skipped,
        unrecorded: count(|outcome| {
            matches!(
                outcome.persistence(),
                Some(PrPersistence::NotRecorded { .. })
            )
        }),
        cancelled,
        committed: rows.iter().any(|row| {
            matches!(
                row.outcome.persistence(),
                Some(PrPersistence::Recorded {
                    write: PrRefreshWriteReport::Applied
                })
            )
        }),
        rows,
    }
}
