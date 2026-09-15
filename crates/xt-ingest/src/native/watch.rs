//! Live tail of native history.
//!
//! A filesystem watcher is registered *before* the initial scan, so a change
//! made while scanning is queued rather than lost. After the scan, queued
//! changes are coalesced and reconciled until the queue is quiet, and only
//! then is the tailer ready. From then on each burst of events, after a short
//! quiet period, reconciles the hosts it touched through the same importer
//! the scan used: Claude files are enumerated and proven against their
//! checkpoints (an unchanged file costs a stat and a short read), reader hosts
//! are read whole again through the pinned producer, whose output dedupes
//! into the index. Event kinds are never trusted: a directory-level or
//! coalesced event marks its host dirty, and the reconciliation decides what
//! actually changed. A watcher that cannot be registered leaves the tailer
//! degraded and says so; it never reports ready as if it were live.
//!
//! Stopping is graceful by default: changes already delivered are reconciled
//! first. A shutdown cancels instead: the reader running is killed and reaped,
//! a Claude scan ends between files, and the worker is joined within a
//! bound, so a producer that never returns cannot hold the process that
//! is exiting. What a cancelled scan had committed stays; the next scan
//! reads the rest.

use super::{
    CancelToken, HostReport, HostStatus, ImportReport, ImportRequest, ProducerSource, ScanMode,
    SessionOutcome, all_imported, scan_native_observed,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use xt_store::{Host, Store};

/// How long a dropped tailer waits for its cancelled worker to finish before
/// leaving the thread behind.
pub const SHUTDOWN_BOUND: Duration = Duration::from_secs(5);

pub struct WatchConfig {
    pub home: PathBuf,
    pub hosts: Vec<Host>,
    /// Where the pinned producer's sources are, for the reader hosts.
    pub producer: ProducerSource,
    pub python: Option<OsString>,
    /// Quiet period after the last event before a burst is reconciled.
    pub debounce: Duration,
    /// Instrumentation: called at the named points on the worker (tests, and
    /// progress reporting during the initial scan).
    pub probe: Option<Probe>,
}

pub type Probe = Arc<dyn Fn(ProbePoint<'_>) + Send + Sync>;

#[derive(Debug)]
pub enum ProbePoint<'a> {
    /// The watcher is registered (or has failed); the initial scan starts next.
    WatcherRegistered,
    /// A watch pass listed the roots to watch; the missing watches are
    /// installed next.
    WatchesDecided,
    /// One Claude file of the initial scan is done.
    FileScanned(&'a Path),
    /// The initial scan is done; queued changes are reconciled next.
    InitialScanDone,
    /// A reconciliation of these hosts is about to run.
    Reconciling(&'a [Host]),
}

/// Whether live changes reach the index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "freshness", rename_all = "snake_case")]
pub enum Freshness {
    Live,
    /// The watcher could not be registered: the index reflects the last scan only.
    Degraded {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Readiness {
    #[serde(flatten)]
    pub freshness: Freshness,
    /// The initial scan, with every host reconciled during startup replaced by
    /// its reconciliation, the records the initial scan indexed counted in
    /// and a session it indexed, or a diagnostic it raised, that the
    /// reconciliation no longer saw kept (a host stays incomplete around such
    /// a session that was not imported, or such a diagnostic).
    pub report: ImportReport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// Changes queued while the initial scan ran.
    Startup,
    /// A burst of change events.
    Change,
    /// The watcher asked for a rescan (an overflow, say): every host.
    Rescan,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TailEvent {
    Ready(Readiness),
    /// Carries the freshness as of this reconciliation: a root that could not
    /// be watched when it appeared degrades it after readiness.
    Reconciled {
        trigger: Trigger,
        #[serde(flatten)]
        freshness: Freshness,
        report: ImportReport,
    },
    /// The final freshness, so a consumer that decides on it need not have
    /// tracked every reconciliation.
    Stopped {
        #[serde(flatten)]
        freshness: Freshness,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct TailStatus {
    #[serde(flatten)]
    pub freshness: Freshness,
    pub ready: bool,
    /// No event is being processed and none was queued when the worker last
    /// looked, after its last reconciliation.
    pub idle: bool,
    /// Reconciliations completed so far: the initial scan, the startup
    /// passes and every later one.
    pub reconciles: u64,
    pub watched: Vec<String>,
    /// Watches installed so far; a replaced root counts again.
    pub watch_installs: u64,
    pub last_error: Option<String>,
    /// The worker has stopped (after a stop request, or on its own).
    pub stopped: bool,
}

enum Message {
    Fs(notify::Result<notify::Event>),
    Stop,
}

#[derive(Default)]
struct State {
    ready: Option<Readiness>,
    freshness: Option<Freshness>,
    /// True only while every event delivered so far has been taken off the
    /// queue and reconciled: the delivery count below and this flag change
    /// under the one lock (a delivery clears the flag as it is counted), so
    /// a waiter never sees idle with a delivered event unprocessed.
    idle: bool,
    /// Filesystem events (and injected ones) handed to the worker's queue.
    delivered: u64,
    reconciles: u64,
    /// Each watched root with the identity of the directory the watch was
    /// installed on, so a root replaced at the same path is watched again.
    watched: BTreeMap<PathBuf, (u64, u64)>,
    watch_installs: u64,
    last_error: Option<String>,
    stopped: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn update(&self, change: impl FnOnce(&mut State)) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        change(&mut state);
        self.changed.notify_all();
    }

    fn wait_until(&self, timeout: Duration, done: impl Fn(&State) -> bool) -> bool {
        // A timeout too large to represent as an instant waits without one.
        let deadline = Instant::now().checked_add(timeout);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if done(&state) {
                return true;
            }
            state = match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return false;
                    }
                    self.changed
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .0
                }
                None => self
                    .changed
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            };
        }
    }
}

pub struct Tailer {
    control: Sender<Message>,
    shared: Arc<Shared>,
    cancel: CancelToken,
    /// Signalled when the worker thread ends, however it ends, so a join can
    /// be bounded.
    finished: Receiver<()>,
    worker: Option<JoinHandle<()>>,
}

/// Sends on drop, so the worker's end is signalled even if it panicked.
struct Finished(Sender<()>);
impl Drop for Finished {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

impl Tailer {
    /// Register the watcher, run the initial scan, reconcile what changed
    /// meanwhile, report ready, then reconcile live changes until stopped.
    /// The store is owned by the worker thread from here on.
    pub fn start(store: Store, config: WatchConfig, sink: Box<dyn Fn(TailEvent) + Send>) -> Self {
        let (tx, rx) = mpsc::channel();
        let (finished_tx, finished) = mpsc::channel();
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let cancel = CancelToken::new();
        let worker = {
            let shared = Arc::clone(&shared);
            let tx = tx.clone();
            let cancel = cancel.clone();
            std::thread::Builder::new()
                .name("xtrace-native-tail".into())
                .spawn(move || {
                    let _finished = Finished(finished_tx);
                    Worker::new(store, config, sink, shared, tx, cancel).run(rx)
                })
                .expect("spawn the native tail worker")
        };
        Self {
            control: tx,
            shared,
            cancel,
            finished,
            worker: Some(worker),
        }
    }

    pub fn status(&self) -> TailStatus {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        TailStatus {
            freshness: state.freshness.clone().unwrap_or(Freshness::Degraded {
                reason: "watcher not registered yet".into(),
            }),
            ready: state.ready.is_some(),
            idle: state.idle,
            reconciles: state.reconciles,
            watched: state
                .watched
                .keys()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            watch_installs: state.watch_installs,
            stopped: state.stopped,
            last_error: state.last_error.clone(),
        }
    }

    /// The readiness report once startup reconciliation is done.
    pub fn wait_ready(&self, timeout: Duration) -> Option<Readiness> {
        if !self
            .shared
            .wait_until(timeout, |state| state.ready.is_some() || state.stopped)
        {
            return None;
        }
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .ready
            .clone()
    }

    /// Wait until at least `count` reconciliations completed and the worker is
    /// idle. Events the platform has not delivered yet are not waited for. A
    /// worker that stopped wakes the waiter, but the wait is met only if the
    /// count was reached: a stop short of it reads as unmet.
    pub fn wait_reconciled(&self, count: u64, timeout: Duration) -> bool {
        let woke = self.shared.wait_until(timeout, |state| {
            state.stopped || (state.reconciles >= count && state.idle)
        });
        woke && {
            let state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.reconciles >= count && state.idle
        }
    }

    /// Deliver a watcher error as the platform would, for tests of the
    /// rebuild path.
    #[doc(hidden)]
    pub fn inject_watcher_error(&self, reason: &str) {
        let control = self.control.clone();
        self.shared.update(|state| {
            state.delivered += 1;
            state.idle = false;
            let _ = control.send(Message::Fs(Err(notify::Error::generic(reason))));
        });
    }

    /// Deliver a synthetic removal event for `path`, as the platform would
    /// report a watched root deleted (for tests).
    pub fn inject_removed(&self, path: &Path) {
        let event =
            notify::Event::new(notify::EventKind::Remove(notify::event::RemoveKind::Folder))
                .add_path(path.to_path_buf());
        let control = self.control.clone();
        self.shared.update(|state| {
            state.delivered += 1;
            state.idle = false;
            let _ = control.send(Message::Fs(Ok(event)));
        });
    }

    /// Queue a stop request without waiting for the worker; `stop` still
    /// waits for it. Changes delivered before the request is honored are
    /// reconciled first.
    pub fn request_stop(&self) {
        let _ = self.control.send(Message::Stop);
    }

    /// Stop gracefully: changes delivered before the request are reconciled
    /// first, and the scan in progress runs to its end. A reader that never
    /// returns holds this call; `shutdown` cancels instead.
    pub fn stop(mut self) {
        let _ = self.control.send(Message::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    /// Cancel the scan in progress and every later one: the reader running is
    /// killed and reaped, a Claude scan ends between files, and each host not
    /// read completely is reported as cancelled. What was committed stays.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// The token the worker's readers and interpreter probes register with,
    /// so a caller's own child processes (an interpreter discovery, say) are
    /// cancelled together with the tailer.
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    /// Cancel, stop, and wait at most `bound` for the worker to end. Returns
    /// false if it had not ended within the bound (it is then left behind,
    /// its reader already killed, and ends with the process).
    pub fn shutdown(mut self, bound: Duration) -> bool {
        self.shutdown_in_place(bound)
    }

    fn shutdown_in_place(&mut self, bound: Duration) -> bool {
        self.cancel.cancel();
        let _ = self.control.send(Message::Stop);
        let Some(worker) = self.worker.take() else {
            return true;
        };
        match self.finished.recv_timeout(bound) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                let _ = worker.join();
                true
            }
            Err(RecvTimeoutError::Timeout) => false,
        }
    }
}

impl Drop for Tailer {
    fn drop(&mut self) {
        self.shutdown_in_place(SHUTDOWN_BOUND);
    }
}

/// A host's startup reconciliation stands in the readiness report for its
/// initial scan, with the records that scan indexed counted in: a session both
/// passes imported reports the sum of their new and enriched records, under
/// the later pass's status, and a session the later pass no longer saw (its
/// file renamed or removed since) stays as the earlier pass reported it, its
/// records being indexed; a diagnostic of the earlier pass that the later
/// pass neither repeated nor resolved (by importing the session at its very
/// path; a directory's diagnostic is never resolved by what was imported
/// below it, which need not be the source it hid) stays as well; a session
/// the earlier pass could not import fully keeps that failure even if the
/// later pass imported it, since the source may have been replaced in
/// between and what the failure left unread is then gone (the counts still
/// add up); a session the earlier pass imported that the later pass could
/// not read reads partial, the earlier counts carried and the later failure
/// as its rejection, since its rows stay indexed; a host the earlier pass could not scan at all (its reader or
/// runtime failed, its pin mismatched, or it was incomplete with no session
/// or diagnostic to say why) stays incomplete even if the later pass
/// completed, since what the failed pass missed may be gone, while an
/// incompleteness that sessions or diagnostics explain is decided by their
/// retention or resolution; and a host with a retained session that was not
/// imported, or a retained diagnostic, stays incomplete, as the earlier pass
/// reported it, so the report never reads complete around a gap. The report
/// then reads the same whether or not the
/// platform also delivered an event for a change made just before the watch
/// was registered (FSEvents may), which queues a second pass that finds the
/// file unchanged.
fn carry_counts(earlier: &HostReport, mut later: HostReport) -> HostReport {
    for session in &mut later.sessions {
        let counted = earlier
            .sessions
            .iter()
            .find(|known| {
                known.native_session_id == session.native_session_id && known.path == session.path
            })
            .and_then(|known| match &known.outcome {
                SessionOutcome::Imported {
                    records_new,
                    records_enriched,
                }
                | SessionOutcome::Partial {
                    records_new,
                    records_enriched,
                    ..
                } => Some((*records_new, *records_enriched)),
                SessionOutcome::Skipped { .. } => None,
            });
        if let Some((new, enriched)) = counted {
            match &mut session.outcome {
                SessionOutcome::Imported {
                    records_new,
                    records_enriched,
                }
                | SessionOutcome::Partial {
                    records_new,
                    records_enriched,
                    ..
                } => {
                    *records_new += new;
                    *records_enriched += enriched;
                }
                SessionOutcome::Skipped { .. } => {}
            }
        }
        // The later pass could not read a session the earlier pass had
        // imported (unreadable since, say): its rows stay indexed, so the
        // merged result is partial, the earlier counts carried and the later
        // failure as its rejection, rather than a skip that hides the rows.
        if let (Some((new, enriched)), SessionOutcome::Skipped { reason }) =
            (counted, &session.outcome)
        {
            // An earlier partial outcome keeps what it could not index too.
            let (records_dropped, mut rejections) = earlier
                .sessions
                .iter()
                .find(|known| {
                    known.native_session_id == session.native_session_id
                        && known.path == session.path
                })
                .and_then(|known| match &known.outcome {
                    SessionOutcome::Partial {
                        records_dropped,
                        rejections,
                        ..
                    } => Some((*records_dropped, rejections.clone())),
                    _ => None,
                })
                .unwrap_or((0, Vec::new()));
            rejections.push(format!("startup pass skipped: {reason}"));
            session.outcome = SessionOutcome::Partial {
                records_new: new,
                records_enriched: enriched,
                records_dropped,
                rejections,
            };
        }
        let failed_earlier = earlier.sessions.iter().find(|known| {
            known.native_session_id == session.native_session_id
                && known.path == session.path
                && !matches!(known.outcome, SessionOutcome::Imported { .. })
        });
        if let (
            Some(known),
            SessionOutcome::Imported {
                records_new,
                records_enriched,
            },
        ) = (failed_earlier, &session.outcome)
        {
            // The later pass imported what it saw, which need not be what
            // the earlier failure left unread: the failure stands, with the
            // records both passes indexed counted.
            session.outcome = match &known.outcome {
                SessionOutcome::Partial {
                    records_dropped,
                    rejections,
                    ..
                } => SessionOutcome::Partial {
                    records_new: *records_new,
                    records_enriched: *records_enriched,
                    records_dropped: *records_dropped,
                    rejections: rejections.clone(),
                },
                other => other.clone(),
            };
        }
    }
    for known in &earlier.sessions {
        let seen = later.sessions.iter().any(|session| {
            session.native_session_id == known.native_session_id && session.path == known.path
        });
        if !seen {
            later.sessions.push(known.clone());
        }
    }
    for diagnostic in &earlier.diagnostics {
        let repeated = later
            .diagnostics
            .iter()
            .any(|known| known.path == diagnostic.path);
        let resolved = diagnostic.path.is_some()
            && later.sessions.iter().any(|session| {
                matches!(session.outcome, SessionOutcome::Imported { .. })
                    && session.path == diagnostic.path
            });
        if !repeated && !resolved {
            later.diagnostics.push(diagnostic.clone());
        }
    }
    // A host-wide failure of the earlier pass: one its reader, runtime or pin
    // reported, or an incomplete status without session or diagnostic
    // evidence. Incompleteness that came from sessions or diagnostics is
    // carried by those, retained or resolved above, not by the status.
    let failed_earlier = matches!(
        earlier.status,
        HostStatus::ReaderFailed
            | HostStatus::MissingRuntime
            | HostStatus::PinMismatch
            | HostStatus::Cancelled
    ) || (earlier.status == HostStatus::Incomplete
        && earlier.sessions.is_empty()
        && earlier.diagnostics.is_empty());
    if later.status == HostStatus::Complete
        && (failed_earlier || !all_imported(&later.sessions) || !later.diagnostics.is_empty())
    {
        later.status = HostStatus::Incomplete;
        if failed_earlier && later.detail.is_none() {
            later.detail = earlier
                .detail
                .as_ref()
                .map(|detail| format!("initial scan: {detail}"));
        }
    }
    later
}

#[derive(Default)]
struct Dirty {
    hosts: Vec<Host>,
    rescan: bool,
    stop: bool,
    /// The watcher reported an error: some installed watch may be lost.
    watcher_failed: Option<String>,
    /// Watched roots an event removed, renamed or recreated (or an ancestor
    /// of one): a platform whose watch dies with the directory may have
    /// dropped them, and a recreation may reuse the inode, so they are
    /// re-registered whatever their identity says.
    lost_watches: Vec<PathBuf>,
}

impl Dirty {
    fn mark(&mut self, host: Host) {
        if !self.hosts.contains(&host) {
            self.hosts.push(host);
        }
    }

    fn mark_all(&mut self, hosts: &[Host]) {
        for host in hosts {
            self.mark(*host);
        }
    }
}

struct Worker {
    store: Store,
    config: WatchConfig,
    /// The home as configured and as the platform reports it (macOS delivers
    /// resolved paths, `/private/var/…` for `/var/…`), so events match either.
    homes: Vec<PathBuf>,
    sink: Box<dyn Fn(TailEvent) + Send>,
    shared: Arc<Shared>,
    events: Sender<Message>,
    watcher: Option<RecommendedWatcher>,
    /// Roots whose watch an event may have dropped, re-registered next.
    lost: Vec<PathBuf>,
    /// Filesystem events taken off the queue so far; idle only once it has
    /// caught up with the events delivered.
    consumed: u64,
    cancel: CancelToken,
}

impl Worker {
    fn new(
        store: Store,
        config: WatchConfig,
        sink: Box<dyn Fn(TailEvent) + Send>,
        shared: Arc<Shared>,
        events: Sender<Message>,
        cancel: CancelToken,
    ) -> Self {
        let mut homes = vec![config.home.clone()];
        if let Ok(resolved) = config.home.canonicalize()
            && resolved != config.home
        {
            homes.push(resolved);
        }
        Self {
            store,
            config,
            homes,
            sink,
            shared,
            events,
            watcher: None,
            lost: Vec::new(),
            consumed: 0,
            cancel,
        }
    }

    /// Before a stop is honored, take every counted delivery off the queue.
    /// Counting and queueing happen under one lock, so every event counted
    /// is already queued: nothing is waited for, and an event delivered
    /// before the stop is honored still reaches the index before shutdown.
    fn drain_counted(&mut self, rx: &Receiver<Message>, dirty: &mut Dirty) {
        loop {
            let delivered = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .delivered;
            if self.consumed >= delivered {
                return;
            }
            match rx.try_recv() {
                Ok(Message::Fs(event)) => {
                    self.consumed += 1;
                    if let Some(error) = classify(
                        event,
                        dirty,
                        &self.config,
                        &self.homes,
                        &self.watched_roots(),
                    ) {
                        dirty.watcher_failed = Some(error);
                    }
                }
                Ok(Message::Stop) => {}
                Err(_) => return,
            }
        }
    }

    /// Publish idle only if every delivered event has been taken off the
    /// queue, under the lock the delivery count changes under.
    fn publish_idle(&self) {
        let consumed = self.consumed;
        self.shared
            .update(|state| state.idle = state.delivered == consumed);
    }

    /// The roots watched now, for classifying events against them.
    fn watched_roots(&self) -> Vec<PathBuf> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .watched
            .keys()
            .cloned()
            .collect()
    }

    fn probe(&self, point: ProbePoint<'_>) {
        if let Some(probe) = &self.config.probe {
            probe(point);
        }
    }

    fn run(mut self, rx: Receiver<Message>) {
        self.register();
        self.probe(ProbePoint::WatcherRegistered);
        let hosts = self.config.hosts.clone();
        let probe = self.config.probe.clone();
        let mut report = self.scan(&hosts, &mut |path| {
            if let Some(probe) = &probe {
                probe(ProbePoint::FileScanned(path));
            }
        });
        self.probe(ProbePoint::InitialScanDone);
        // The initial scan is the first reconciliation: a caller waiting for
        // one after ready must not wait for an unrelated change.
        self.shared.update(|state| state.reconciles += 1);
        // Whatever changed while scanning is reconciled before ready, until
        // the queue is quiet; a change during that reconciliation queues again.
        let mut stopped = false;
        loop {
            let mut dirty = Dirty::default();
            self.consumed += drain(
                &rx,
                self.config.debounce,
                &mut dirty,
                &self.config,
                &self.homes,
                &self.watched_roots(),
            );
            if dirty.stop {
                self.drain_counted(&rx, &mut dirty);
            }
            if let Some(reason) = dirty.watcher_failed.take() {
                self.rebuild_watches(reason);
            }
            self.lost.append(&mut dirty.lost_watches);
            // A root that appeared after the pass that decided the watches
            // was enumerated without a watch of its own, and its creation
            // was not necessarily reported: it is watched now and its host
            // reconciled again before ready.
            for host in self.ensure_watches() {
                dirty.mark(host);
            }
            // Changes received before a stop request are still reconciled.
            if !dirty.hosts.is_empty() {
                let hosts = dirty.hosts.clone();
                let reconciled = self.reconcile(&hosts);
                absorb(&mut report, &reconciled);
                self.finished(
                    if dirty.rescan {
                        Trigger::Rescan
                    } else {
                        Trigger::Startup
                    },
                    reconciled,
                );
            }
            if dirty.stop {
                stopped = true;
                break;
            }
            if !dirty.hosts.is_empty() {
                continue;
            }
            // Ready is published only if nothing is queued at that moment,
            // checked under the lock deliveries are counted under: an event
            // delivered after the drain went quiet repeats the loop instead,
            // so ready never claims an index a delivered change is missing.
            let readiness = Readiness {
                freshness: self.freshness(),
                report: report.clone(),
            };
            let consumed = self.consumed;
            let published = {
                let mut state = self
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if state.delivered == consumed {
                    state.ready = Some(readiness.clone());
                    state.idle = true;
                    self.shared.changed.notify_all();
                    true
                } else {
                    false
                }
            };
            if published {
                (self.sink)(TailEvent::Ready(readiness));
                break;
            }
        }
        if !stopped {
            // Live: each burst, after a quiet period, reconciles its hosts.
            let mut pending: Option<Message> = None;
            loop {
                let message = match pending.take() {
                    Some(message) => message,
                    None => match rx.recv() {
                        Ok(message) => message,
                        Err(_) => break,
                    },
                };
                self.shared.update(|state| state.idle = false);
                let mut dirty = Dirty::default();
                match message {
                    // A stop still lets already-delivered changes, and any
                    // that arrive within one quiet period, reach the index.
                    Message::Stop => dirty.stop = true,
                    Message::Fs(event) => {
                        self.consumed += 1;
                        if let Some(error) = classify(
                            event,
                            &mut dirty,
                            &self.config,
                            &self.homes,
                            &self.watched_roots(),
                        ) {
                            dirty.watcher_failed = Some(error);
                        }
                    }
                }
                self.consumed += drain(
                    &rx,
                    self.config.debounce,
                    &mut dirty,
                    &self.config,
                    &self.homes,
                    &self.watched_roots(),
                );
                if dirty.stop {
                    self.drain_counted(&rx, &mut dirty);
                }
                if let Some(reason) = dirty.watcher_failed.take() {
                    self.rebuild_watches(reason);
                }
                self.lost.append(&mut dirty.lost_watches);
                // A lost watch is registered again even when the event that
                // lost it named no host (the home itself replaced, say), and
                // the host whose root it is, is reconciled.
                for host in self.ensure_watches() {
                    dirty.mark(host);
                }
                if !dirty.hosts.is_empty() {
                    let hosts = dirty.hosts.clone();
                    let reconciled = self.reconcile(&hosts);
                    self.finished(
                        if dirty.rescan {
                            Trigger::Rescan
                        } else {
                            Trigger::Change
                        },
                        reconciled,
                    );
                }
                if dirty.stop {
                    break;
                }
                // A change delivered while the reconciliation ran is still
                // queued, or about to be: the worker is idle only once it has
                // taken every delivered event off the queue (a count that
                // changes under the same lock as the flag), so a waiter never
                // wakes with a delivered change unprocessed.
                match rx.try_recv() {
                    Ok(message) => pending = Some(message),
                    Err(std::sync::mpsc::TryRecvError::Empty) => self.publish_idle(),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
        }
        self.shared.update(|state| {
            state.stopped = true;
            state.idle = true;
        });
        (self.sink)(TailEvent::Stopped {
            freshness: self.freshness(),
        });
    }

    fn finished(&mut self, trigger: Trigger, report: ImportReport) {
        self.shared.update(|state| state.reconciles += 1);
        (self.sink)(TailEvent::Reconciled {
            trigger,
            freshness: self.freshness(),
            report,
        });
    }

    /// Reconcile the given hosts. A root that appeared since the last watch
    /// pass is watched *before* it is enumerated, so a file created below it
    /// after enumeration still produces an event. A root that appeared during
    /// the pass, after its watches were decided, is watched afterwards and
    /// its host scanned again, until a pass finds every root watched: no
    /// change below a root falls between its enumeration and its watch.
    fn reconcile(&mut self, hosts: &[Host]) -> ImportReport {
        self.probe(ProbePoint::Reconciling(hosts));
        self.ensure_watches();
        let mut report = self.scan(hosts, &mut |_| {});
        loop {
            let late = self.ensure_watches();
            if late.is_empty() {
                return report;
            }
            let again = self.scan(&late, &mut |_| {});
            absorb(&mut report, &again);
        }
    }

    fn freshness(&self) -> Freshness {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .freshness
            .clone()
            .unwrap_or(Freshness::Degraded {
                reason: "watcher not registered".into(),
            })
    }

    fn scan(&mut self, hosts: &[Host], observer: &mut dyn FnMut(&Path)) -> ImportReport {
        let request = ImportRequest {
            home: &self.config.home,
            hosts,
            producer: &self.config.producer,
            python: self.config.python.as_deref(),
            observed_at: now_ms(),
            cancel: Some(&self.cancel),
        };
        scan_native_observed(&mut self.store, &request, ScanMode::Resume, observer)
    }

    /// Create the watcher and watch every host root that exists; a root that
    /// does not exist yet is covered by the nearest existing ancestor, so its
    /// creation is seen and it gets its own watch on the next reconciliation.
    fn register(&mut self) {
        let events = self.events.clone();
        let shared = Arc::clone(&self.shared);
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            // Counted and queued under the one lock: an event counted is an
            // event queued, so the worker drains every counted delivery
            // without waiting, and can never have taken more off the queue
            // than was delivered.
            shared.update(|state| {
                state.delivered += 1;
                state.idle = false;
                let _ = events.send(Message::Fs(event));
            });
        });
        match watcher {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                self.shared
                    .update(|state| state.freshness = Some(Freshness::Live));
                self.ensure_watches();
            }
            Err(error) => {
                let reason = format!("filesystem watcher could not be created: {error}");
                self.shared.update(|state| {
                    state.freshness = Some(Freshness::Degraded {
                        reason: reason.clone(),
                    });
                    state.last_error = Some(reason);
                });
            }
        }
    }

    /// The watcher reported an error, so any installed watch may be lost:
    /// every watch is dropped and freshness degraded until the next pass
    /// installs them again (a rescan of every host follows the error).
    fn rebuild_watches(&mut self, reason: String) {
        let watched = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .watched
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        if let Some(watcher) = self.watcher.as_mut() {
            for path in &watched {
                let _ = watcher.unwatch(path);
            }
        }
        let reason = format!("watcher reported an error: {reason}");
        self.shared.update(|state| {
            state.watched.clear();
            state.freshness = Some(Freshness::Degraded {
                reason: reason.clone(),
            });
            state.last_error = Some(reason);
        });
    }

    /// Install the watches still missing; returns the hosts whose own root
    /// gained a watch in this pass, which must be scanned again if a scan ran
    /// since the roots were listed.
    fn ensure_watches(&mut self) -> Vec<Host> {
        let mut late = Vec::new();
        if self.watcher.is_none() {
            return late;
        }
        let roots = watch_roots(&self.config.home, &self.config.hosts);
        self.probe(ProbePoint::WatchesDecided);
        let Some(watcher) = self.watcher.as_mut() else {
            return late;
        };
        let mut failed = false;
        // A watched root that vanished, or that is no longer the directory
        // the watch was installed on (deleted and recreated at the same path
        // before this pass), is forgotten and its watch dropped: a platform
        // whose watch stays tied to the old inode would otherwise stay blind
        // to the replacement. The nearest existing ancestor covers it until
        // it is watched again below.
        let stale = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .watched
            .iter()
            .filter(|(path, identity)| directory_identity(path) != Some(**identity))
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        for path in stale {
            let _ = watcher.unwatch(&path);
            self.shared.update(|state| {
                state.watched.remove(&path);
            });
        }
        // A root an event removed, renamed or recreated is registered again
        // whatever its identity says: a platform whose watch dies with the
        // directory may have dropped it, and a recreation may reuse the inode.
        for path in std::mem::take(&mut self.lost) {
            let _ = watcher.unwatch(&path);
            self.shared.update(|state| {
                state.watched.remove(&path);
            });
        }
        for root in roots {
            let already = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .watched
                .contains_key(&root.path);
            if already {
                continue;
            }
            let Some(identity) = directory_identity(&root.path) else {
                // Nothing covers this root: not even its nearest ancestor exists.
                let reason = format!(
                    "{} could not be watched: it is absent or not a directory",
                    root.path.display()
                );
                failed = true;
                self.shared.update(|state| {
                    state.freshness = Some(Freshness::Degraded {
                        reason: reason.clone(),
                    });
                    state.last_error = Some(reason);
                });
                continue;
            };
            let mode = if root.recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            match watcher.watch(&root.path, mode) {
                Ok(()) => {
                    self.shared.update(|state| {
                        state.watched.insert(root.path.clone(), identity);
                        state.watch_installs += 1;
                    });
                    if let Some(host) = root.host
                        && !late.contains(&host)
                    {
                        late.push(host);
                    }
                }
                Err(error) => {
                    let reason = format!("{} could not be watched: {error}", root.path.display());
                    failed = true;
                    self.shared.update(|state| {
                        state.freshness = Some(Freshness::Degraded {
                            reason: reason.clone(),
                        });
                        state.last_error = Some(reason);
                    });
                }
            }
        }
        // Freshness reflects the current watches: every root covered again
        // after a rebuild or a late arrival makes the tailer live again.
        if !failed {
            self.shared
                .update(|state| state.freshness = Some(Freshness::Live));
        }
        late
    }
}

/// Fold a later pass into `report`: each host it reconciled stands in for the
/// earlier report of that host, with the earlier counts carried.
fn absorb(report: &mut ImportReport, later: &ImportReport) {
    for host in &later.hosts {
        match report
            .hosts
            .iter_mut()
            .find(|known| known.host == host.host)
        {
            Some(known) => *known = carry_counts(known, host.clone()),
            None => report.hosts.push(host.clone()),
        }
    }
}

struct WatchRoot {
    path: PathBuf,
    recursive: bool,
    /// The host whose own root this is; none for a fallback watch on an
    /// ancestor, which only reports roots appearing below it.
    host: Option<Host>,
}

/// The device and inode of a directory, without following an alias; `None`
/// when it is absent or not a directory. Platforms without inodes identify
/// every directory alike, so only absence is detected there.
fn directory_identity(path: &Path) -> Option<(u64, u64)> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        Some((0, 0))
    }
}

/// The deepest existing directory covering each host's sources. The home
/// itself is watched without recursion when a host root is absent, so the
/// root's creation is noticed.
fn watch_roots(home: &Path, hosts: &[Host]) -> Vec<WatchRoot> {
    // The home may be an accepted alias (a symlinked home directory), unlike
    // a source root: the fallback watch goes on the directory it names.
    let fallback = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    fn add(roots: &mut Vec<WatchRoot>, fallback: &Path, host: Host, candidates: &[PathBuf]) {
        for candidate in candidates {
            if candidate.is_dir() {
                if !roots.iter().any(|root| root.path == *candidate) {
                    roots.push(WatchRoot {
                        path: candidate.clone(),
                        recursive: true,
                        host: Some(host),
                    });
                }
                return;
            }
        }
        if !roots.iter().any(|root| root.path == fallback) {
            roots.push(WatchRoot {
                path: fallback.to_path_buf(),
                recursive: false,
                host: None,
            });
        }
    }
    let mut roots: Vec<WatchRoot> = Vec::new();
    for host in hosts {
        match host {
            Host::Claude => add(
                &mut roots,
                &fallback,
                *host,
                &[home.join(".claude/projects"), home.join(".claude")],
            ),
            Host::Codex => add(
                &mut roots,
                &fallback,
                *host,
                &[home.join(".codex/sessions"), home.join(".codex")],
            ),
            Host::Cursor => {
                // Two roots may exist independently; the parent is watched
                // without recursion so the absent one is seen when it appears.
                let parent = home.join(".cursor");
                let chats = home.join(".cursor/chats");
                let projects = home.join(".cursor/projects");
                if chats.is_dir() || projects.is_dir() {
                    for root in [chats, projects] {
                        if root.is_dir() {
                            add(&mut roots, &fallback, *host, &[root]);
                        }
                    }
                    if parent.is_dir() && !roots.iter().any(|root| root.path == parent) {
                        roots.push(WatchRoot {
                            path: parent,
                            recursive: false,
                            host: None,
                        });
                    }
                } else {
                    add(&mut roots, &fallback, *host, &[parent]);
                }
                // The hook's state pins fold into a Cursor session's clock and
                // usage, and change on their own: their directory is watched
                // like a root, or its nearest existing ancestor (without
                // recursion) until it appears.
                let pins = home.join(".config/memhub-plugin/cursorflush");
                if pins.is_dir() {
                    add(&mut roots, &fallback, *host, &[pins]);
                } else {
                    let ancestor = [home.join(".config/memhub-plugin"), home.join(".config")]
                        .into_iter()
                        .find(|dir| dir.is_dir())
                        .unwrap_or_else(|| fallback.clone());
                    if !roots.iter().any(|root| root.path == ancestor) {
                        roots.push(WatchRoot {
                            path: ancestor,
                            recursive: false,
                            host: None,
                        });
                    }
                }
            }
            Host::Other => {}
        }
    }
    roots
}

/// The host a changed path belongs to, by its position under the home (in
/// any spelling the platform may report it). The Cursor hook's state pins
/// under `.config/memhub-plugin/cursorflush` belong to Cursor, as do that
/// directory's ancestors appearing.
fn host_of(homes: &[PathBuf], path: &Path) -> Option<Host> {
    let relative = homes.iter().find_map(|home| path.strip_prefix(home).ok())?;
    let mut components = relative
        .components()
        .map(|component| component.as_os_str().to_str());
    match components.next()?? {
        ".claude" => Some(Host::Claude),
        ".codex" => Some(Host::Codex),
        ".cursor" => Some(Host::Cursor),
        ".config" => match components.next() {
            None | Some(Some("memhub-plugin")) => Some(Host::Cursor),
            _ => None,
        },
        _ => None,
    }
}

/// Note what an event touched. Kinds are not trusted for dirtying: any path
/// under a host root marks that host, and a rescan request or watcher error
/// marks every host. A removal, rename, folder creation or unclassified
/// event at a watched root or above it also marks that root's watch lost.
/// Returns the watcher's error text, if it reported one.
fn classify(
    event: notify::Result<notify::Event>,
    dirty: &mut Dirty,
    config: &WatchConfig,
    homes: &[PathBuf],
    watched: &[PathBuf],
) -> Option<String> {
    use notify::EventKind;
    match event {
        Ok(event) => {
            if event.need_rescan() {
                dirty.rescan = true;
                dirty.mark_all(&config.hosts);
            }
            let may_drop_watch = matches!(
                event.kind,
                EventKind::Remove(_)
                    | EventKind::Modify(notify::event::ModifyKind::Name(_))
                    | EventKind::Create(notify::event::CreateKind::Folder)
                    | EventKind::Any
                    | EventKind::Other
            );
            for path in &event.paths {
                if let Some(host) = host_of(homes, path)
                    && config.hosts.contains(&host)
                {
                    dirty.mark(host);
                }
                if may_drop_watch {
                    for root in watched {
                        if root.starts_with(path) && !dirty.lost_watches.contains(root) {
                            dirty.lost_watches.push(root.clone());
                        }
                    }
                }
            }
            None
        }
        Err(error) => {
            // The watcher lost track of something: every host is reconciled.
            dirty.rescan = true;
            dirty.mark_all(&config.hosts);
            Some(error.to_string())
        }
    }
}

/// Collect queued events until the channel stays quiet for `debounce`;
/// returns how many filesystem events were taken off the queue.
fn drain(
    rx: &Receiver<Message>,
    debounce: Duration,
    dirty: &mut Dirty,
    config: &WatchConfig,
    homes: &[PathBuf],
    watched: &[PathBuf],
) -> u64 {
    let mut consumed = 0;
    loop {
        match rx.recv_timeout(debounce) {
            Ok(Message::Fs(event)) => {
                consumed += 1;
                if let Some(error) = classify(event, dirty, config, homes, watched) {
                    dirty.watcher_failed = Some(error);
                }
            }
            Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => {
                dirty.stop = true;
                return consumed;
            }
            Err(RecvTimeoutError::Timeout) => return consumed,
        }
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or_default(),
    )
    .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::super::{ReaderDiagnostic, SessionResult};
    use super::*;

    #[test]
    fn a_resolved_diagnostic_completes_a_host_a_status_only_failure_does_not() {
        let session = |path: &str, outcome: SessionOutcome| SessionResult {
            native_session_id: Some("s".into()),
            conversation_id: Some("s".into()),
            source_surface: None,
            path: Some(path.into()),
            outcome,
        };
        let diagnostic = |path: &str| ReaderDiagnostic {
            code: "discovery_incomplete".into(),
            path: Some(path.into()),
        };
        let host = |status: HostStatus,
                    detail: Option<&str>,
                    diagnostics: Vec<ReaderDiagnostic>,
                    sessions: Vec<SessionResult>| HostReport {
            host: Host::Claude,
            status,
            detail: detail.map(str::to_owned),
            diagnostics,
            sessions,
        };
        let imported = || SessionOutcome::Imported {
            records_new: 2,
            records_enriched: 0,
        };
        // Incomplete only because of a diagnostic the later pass resolved by
        // importing the session at that very path: complete.
        let earlier = host(
            HostStatus::Incomplete,
            None,
            vec![diagnostic("/h/p/s.jsonl")],
            vec![],
        );
        let later = host(
            HostStatus::Complete,
            None,
            vec![],
            vec![session("/h/p/s.jsonl", imported())],
        );
        let merged = carry_counts(&earlier, later);
        assert_eq!(merged.status, HostStatus::Complete, "{merged:?}");
        assert!(merged.diagnostics.is_empty());
        // Incomplete with nothing to say why (a host-wide failure): stays so,
        // with the reason carried.
        let earlier = host(
            HostStatus::Incomplete,
            Some("root unavailable"),
            vec![],
            vec![],
        );
        let later = host(
            HostStatus::Complete,
            None,
            vec![],
            vec![session("/h/p/s.jsonl", imported())],
        );
        let merged = carry_counts(&earlier, later);
        assert_eq!(merged.status, HostStatus::Incomplete);
        assert_eq!(
            merged.detail.as_deref(),
            Some("initial scan: root unavailable")
        );
        // A reader failure likewise.
        let earlier = host(
            HostStatus::ReaderFailed,
            Some("producer died"),
            vec![],
            vec![],
        );
        let later = host(HostStatus::Complete, None, vec![], vec![]);
        assert_eq!(carry_counts(&earlier, later).status, HostStatus::Incomplete);
        // A session the earlier pass imported that the later pass skipped:
        // partial, the earlier counts carried, the later failure kept.
        let earlier = host(
            HostStatus::Complete,
            None,
            vec![],
            vec![session("/h/p/s.jsonl", imported())],
        );
        let later = host(
            HostStatus::Incomplete,
            None,
            vec![],
            vec![session(
                "/h/p/s.jsonl",
                SessionOutcome::Skipped {
                    reason: "file could not be read: PermissionDenied".into(),
                },
            )],
        );
        let merged = carry_counts(&earlier, later);
        assert_eq!(merged.status, HostStatus::Incomplete);
        assert_eq!(
            merged.sessions[0].outcome,
            SessionOutcome::Partial {
                records_new: 2,
                records_enriched: 0,
                records_dropped: 0,
                rejections: vec![
                    "startup pass skipped: file could not be read: PermissionDenied".into()
                ],
            }
        );
        // An earlier partial session skipped by the later pass keeps what it
        // could not index, the later failure added.
        let earlier = host(
            HostStatus::Incomplete,
            None,
            vec![],
            vec![session(
                "/h/p/s.jsonl",
                SessionOutcome::Partial {
                    records_new: 2,
                    records_enriched: 0,
                    records_dropped: 1,
                    rejections: vec!["missing_uuid".into()],
                },
            )],
        );
        let later = host(
            HostStatus::Incomplete,
            None,
            vec![],
            vec![session(
                "/h/p/s.jsonl",
                SessionOutcome::Skipped {
                    reason: "file could not be read: PermissionDenied".into(),
                },
            )],
        );
        let merged = carry_counts(&earlier, later);
        assert_eq!(
            merged.sessions[0].outcome,
            SessionOutcome::Partial {
                records_new: 2,
                records_enriched: 0,
                records_dropped: 1,
                rejections: vec![
                    "missing_uuid".into(),
                    "startup pass skipped: file could not be read: PermissionDenied".into()
                ],
            }
        );
        // A diagnostic the later pass neither repeated nor resolved stays,
        // and keeps the host incomplete.
        let earlier = host(
            HostStatus::Incomplete,
            None,
            vec![diagnostic("/h/q")],
            vec![],
        );
        let later = host(
            HostStatus::Complete,
            None,
            vec![],
            vec![session("/h/q/t.jsonl", imported())],
        );
        let merged = carry_counts(&earlier, later);
        assert_eq!(merged.status, HostStatus::Incomplete);
        assert_eq!(merged.diagnostics, vec![diagnostic("/h/q")]);
    }

    #[test]
    fn a_removal_at_or_above_a_watched_root_marks_its_watch_lost() {
        use notify::{Event, EventKind, event::*};
        let home = PathBuf::from("/h");
        let config = WatchConfig {
            home: home.clone(),
            hosts: vec![Host::Claude],
            producer: ProducerSource::Checkout {
                pin: PathBuf::from("/pin"),
                plugin_root: None,
            },
            python: None,
            debounce: Duration::from_millis(1),
            probe: None,
        };
        let homes = vec![home.clone()];
        let root = home.join(".claude/projects");
        let watched = vec![root.clone()];
        let classify_kind = |kind: EventKind, path: PathBuf| {
            let mut dirty = Dirty::default();
            let event = Event::new(kind).add_path(path);
            classify(Ok(event), &mut dirty, &config, &homes, &watched);
            dirty
        };
        // The root itself removed, renamed or recreated: its watch is lost.
        for kind in [
            EventKind::Remove(RemoveKind::Folder),
            EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            EventKind::Create(CreateKind::Folder),
            EventKind::Any,
        ] {
            let dirty = classify_kind(kind, root.clone());
            assert_eq!(dirty.lost_watches, vec![root.clone()], "{kind:?}");
            assert_eq!(dirty.hosts, vec![Host::Claude]);
        }
        // An ancestor removed takes the root with it.
        let dirty = classify_kind(EventKind::Remove(RemoveKind::Folder), home.join(".claude"));
        assert_eq!(dirty.lost_watches, vec![root.clone()]);
        // A change below the root, or a data change of the root, drops nothing.
        let dirty = classify_kind(EventKind::Remove(RemoveKind::File), root.join("p/a.jsonl"));
        assert!(dirty.lost_watches.is_empty());
        assert_eq!(dirty.hosts, vec![Host::Claude]);
        let dirty = classify_kind(
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            root.clone(),
        );
        assert!(dirty.lost_watches.is_empty());
    }

    #[test]
    fn paths_map_to_hosts_by_their_home_relative_root() {
        let homes = [PathBuf::from("/h"), PathBuf::from("/private/h")];
        assert_eq!(
            host_of(&homes, Path::new("/h/.claude/projects/p/s.jsonl")),
            Some(Host::Claude)
        );
        assert_eq!(
            host_of(&homes, Path::new("/private/h/.claude/projects/p/s.jsonl")),
            Some(Host::Claude)
        );
        assert_eq!(host_of(&homes, Path::new("/h/.codex")), Some(Host::Codex));
        assert_eq!(
            host_of(&homes, Path::new("/h/.cursor/chats/x/store.db")),
            Some(Host::Cursor)
        );
        assert_eq!(host_of(&homes, Path::new("/h/other")), None);
        assert_eq!(host_of(&homes, Path::new("/elsewhere/.claude")), None);
    }

    #[test]
    fn absent_roots_fall_back_to_the_home_without_recursion() {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path();
        std::fs::create_dir_all(home.join(".claude/projects")).unwrap();
        let roots = watch_roots(home, &[Host::Claude, Host::Codex, Host::Cursor]);
        let described = roots
            .iter()
            .map(|root| (root.path.clone(), root.recursive))
            .collect::<Vec<_>>();
        let fallback = home.canonicalize().unwrap();
        assert_eq!(
            described,
            vec![
                (home.join(".claude/projects"), true),
                (fallback.clone(), false)
            ]
        );
        // One Cursor root present: it is watched recursively and the parent
        // without recursion, so the sibling is seen when it appears.
        std::fs::create_dir_all(home.join(".cursor/projects")).unwrap();
        let roots = watch_roots(home, &[Host::Cursor]);
        let described = roots
            .iter()
            .map(|root| (root.path.clone(), root.recursive))
            .collect::<Vec<_>>();
        assert_eq!(
            described,
            vec![
                (home.join(".cursor/projects"), true),
                (home.join(".cursor"), false),
                // The hook's state pins: absent, so the home stands in.
                (fallback.clone(), false)
            ]
        );
        // The pin directory's nearest existing ancestor stands in without
        // recursion until it appears; then it is watched like a root.
        std::fs::create_dir_all(home.join(".config")).unwrap();
        let described = |roots: Vec<WatchRoot>| {
            roots
                .into_iter()
                .map(|root| (root.path, root.recursive, root.host))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            described(watch_roots(home, &[Host::Cursor])).last(),
            Some(&(home.join(".config"), false, None))
        );
        std::fs::create_dir_all(home.join(".config/memhub-plugin")).unwrap();
        assert_eq!(
            described(watch_roots(home, &[Host::Cursor])).last(),
            Some(&(home.join(".config/memhub-plugin"), false, None))
        );
        std::fs::create_dir_all(home.join(".config/memhub-plugin/cursorflush")).unwrap();
        assert_eq!(
            described(watch_roots(home, &[Host::Cursor])).last(),
            Some(&(
                home.join(".config/memhub-plugin/cursorflush"),
                true,
                Some(Host::Cursor)
            ))
        );
        // Pins and their directory's ancestors classify as Cursor; other
        // configuration does not.
        let homes = vec![home.to_path_buf()];
        assert_eq!(
            host_of(
                &homes,
                &home.join(".config/memhub-plugin/cursorflush/x.json")
            ),
            Some(Host::Cursor)
        );
        assert_eq!(
            host_of(&homes, &home.join(".config/memhub-plugin")),
            Some(Host::Cursor)
        );
        assert_eq!(host_of(&homes, &home.join(".config")), Some(Host::Cursor));
        assert_eq!(host_of(&homes, &home.join(".config/gh/hosts.yml")), None);
    }
}
