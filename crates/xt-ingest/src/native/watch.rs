//! Live tail of native history.
//!
//! A filesystem watcher is registered *before* the initial scan, so a change
//! made while scanning is queued rather than lost. After the scan, queued
//! changes are coalesced and reconciled until the queue is quiet, and only
//! then is the tailer ready. From then on each burst of events, after a short
//! quiet period, reconciles the hosts it touched through the same importer
//! the scan used: Claude files are enumerated and proven against their
//! checkpoints (an unchanged file costs a stat and a short read), reader hosts
//! are rescanned through the pinned producer for sessions modified since their
//! last gapless scan. Event kinds are never trusted: a directory-level or
//! coalesced event marks its host dirty, and the reconciliation decides what
//! actually changed. A watcher that cannot be registered leaves the tailer
//! degraded and says so; it never reports ready as if it were live.

use super::{
    HostReport, HostStatus, ImportReport, ImportRequest, ScanMode, SessionOutcome, all_imported,
    scan_native_observed,
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

pub struct WatchConfig {
    pub home: PathBuf,
    pub hosts: Vec<Host>,
    pub pin: PathBuf,
    pub plugin_root: Option<PathBuf>,
    pub python: Option<OsString>,
    /// Quiet period after the last event before a burst is reconciled.
    pub debounce: Duration,
    /// Instrumentation for tests: called at the named points on the worker.
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
    /// No event is being processed and none was pending when the worker last looked.
    pub idle: bool,
    /// Reconciliations completed so far, the startup pass included.
    pub reconciles: u64,
    pub watched: Vec<String>,
    /// Watches installed so far; a replaced root counts again.
    pub watch_installs: u64,
    pub last_error: Option<String>,
}

enum Message {
    Fs(notify::Result<notify::Event>),
    Stop,
}

#[derive(Default)]
struct State {
    ready: Option<Readiness>,
    freshness: Option<Freshness>,
    idle: bool,
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
    worker: Option<JoinHandle<()>>,
}

impl Tailer {
    /// Register the watcher, run the initial scan, reconcile what changed
    /// meanwhile, report ready, then reconcile live changes until stopped.
    /// The store is owned by the worker thread from here on.
    pub fn start(store: Store, config: WatchConfig, sink: Box<dyn Fn(TailEvent) + Send>) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let worker = {
            let shared = Arc::clone(&shared);
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("xtrace-native-tail".into())
                .spawn(move || Worker::new(store, config, sink, shared, tx).run(rx))
                .expect("spawn the native tail worker")
        };
        Self {
            control: tx,
            shared,
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
    /// idle. Events the platform has not delivered yet are not waited for.
    pub fn wait_reconciled(&self, count: u64, timeout: Duration) -> bool {
        self.shared.wait_until(timeout, |state| {
            state.stopped || (state.reconciles >= count && state.idle)
        })
    }

    /// Deliver a watcher error as the platform would, for tests of the
    /// rebuild path.
    #[doc(hidden)]
    pub fn inject_watcher_error(&self, reason: &str) {
        let _ = self
            .control
            .send(Message::Fs(Err(notify::Error::generic(reason))));
    }

    /// Deliver a synthetic removal event for `path`, as the platform would
    /// report a watched root deleted (for tests).
    pub fn inject_removed(&self, path: &Path) {
        let event =
            notify::Event::new(notify::EventKind::Remove(notify::event::RemoveKind::Folder))
                .add_path(path.to_path_buf());
        let _ = self.control.send(Message::Fs(Ok(event)));
    }

    pub fn stop(mut self) {
        let _ = self.control.send(Message::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Tailer {
    fn drop(&mut self) {
        let _ = self.control.send(Message::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
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
/// below it, which need not be the source it hid) stays as well; and a host with a retained session that was not
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
    if later.status == HostStatus::Complete
        && (!all_imported(&later.sessions) || !later.diagnostics.is_empty())
    {
        later.status = HostStatus::Incomplete;
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
}

impl Worker {
    fn new(
        store: Store,
        config: WatchConfig,
        sink: Box<dyn Fn(TailEvent) + Send>,
        shared: Arc<Shared>,
        events: Sender<Message>,
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
        }
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
        // Whatever changed while scanning is reconciled before ready, until
        // the queue is quiet; a change during that reconciliation queues again.
        let mut stopped = false;
        loop {
            let mut dirty = Dirty::default();
            drain(
                &rx,
                self.config.debounce,
                &mut dirty,
                &self.config,
                &self.homes,
                &self.watched_roots(),
            );
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
            if dirty.hosts.is_empty() {
                break;
            }
        }
        if !stopped {
            let readiness = Readiness {
                freshness: self.freshness(),
                report,
            };
            self.shared.update(|state| {
                state.ready = Some(readiness.clone());
                state.idle = true;
            });
            (self.sink)(TailEvent::Ready(readiness));
            // Live: each burst, after a quiet period, reconciles its hosts.
            while let Ok(message) = rx.recv() {
                self.shared.update(|state| state.idle = false);
                let mut dirty = Dirty::default();
                match message {
                    // A stop still lets already-delivered changes, and any
                    // that arrive within one quiet period, reach the index.
                    Message::Stop => dirty.stop = true,
                    Message::Fs(event) => {
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
                drain(
                    &rx,
                    self.config.debounce,
                    &mut dirty,
                    &self.config,
                    &self.homes,
                    &self.watched_roots(),
                );
                if let Some(reason) = dirty.watcher_failed.take() {
                    self.rebuild_watches(reason);
                }
                self.lost.append(&mut dirty.lost_watches);
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
                self.shared.update(|state| state.idle = true);
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
            pin: &self.config.pin,
            plugin_root: self.config.plugin_root.as_deref(),
            python: self.config.python.as_deref(),
            observed_at: now_ms(),
        };
        scan_native_observed(&mut self.store, &request, ScanMode::Resume, observer)
    }

    /// Create the watcher and watch every host root that exists; a root that
    /// does not exist yet is covered by the nearest existing ancestor, so its
    /// creation is seen and it gets its own watch on the next reconciliation.
    fn register(&mut self) {
        let events = self.events.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let _ = events.send(Message::Fs(event));
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
    fn add(roots: &mut Vec<WatchRoot>, home: &Path, host: Host, candidates: &[PathBuf]) {
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
        if !roots.iter().any(|root| root.path == home) {
            roots.push(WatchRoot {
                path: home.to_path_buf(),
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
                home,
                *host,
                &[home.join(".claude/projects"), home.join(".claude")],
            ),
            Host::Codex => add(
                &mut roots,
                home,
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
                            add(&mut roots, home, *host, &[root]);
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
                    add(&mut roots, home, *host, &[parent]);
                }
                // The hook's state pins fold into a Cursor session's clock and
                // stamp, and change on their own: their directory is watched
                // like a root, or its nearest existing ancestor (without
                // recursion) until it appears.
                let pins = home.join(".config/memhub-plugin/cursorflush");
                if pins.is_dir() {
                    add(&mut roots, home, *host, &[pins]);
                } else {
                    let ancestor = [home.join(".config/memhub-plugin"), home.join(".config")]
                        .into_iter()
                        .find(|dir| dir.is_dir())
                        .unwrap_or_else(|| home.to_path_buf());
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

/// Collect queued events until the channel stays quiet for `debounce`.
fn drain(
    rx: &Receiver<Message>,
    debounce: Duration,
    dirty: &mut Dirty,
    config: &WatchConfig,
    homes: &[PathBuf],
    watched: &[PathBuf],
) {
    loop {
        match rx.recv_timeout(debounce) {
            Ok(Message::Fs(event)) => {
                if let Some(error) = classify(event, dirty, config, homes, watched) {
                    dirty.watcher_failed = Some(error);
                }
            }
            Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => {
                dirty.stop = true;
                return;
            }
            Err(RecvTimeoutError::Timeout) => return,
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
    use super::*;

    #[test]
    fn a_removal_at_or_above_a_watched_root_marks_its_watch_lost() {
        use notify::{Event, EventKind, event::*};
        let home = PathBuf::from("/h");
        let config = WatchConfig {
            home: home.clone(),
            hosts: vec![Host::Claude],
            pin: PathBuf::from("/pin"),
            plugin_root: None,
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
        assert_eq!(
            described,
            vec![
                (home.join(".claude/projects"), true),
                (home.to_path_buf(), false)
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
                (home.to_path_buf(), false)
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
