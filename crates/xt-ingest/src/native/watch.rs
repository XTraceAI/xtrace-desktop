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

use super::{ImportReport, ImportRequest, ScanMode, scan_native_observed};
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
    /// its reconciliation.
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

#[derive(Default)]
struct Dirty {
    hosts: Vec<Host>,
    rescan: bool,
    stop: bool,
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
        }
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
                &self.shared,
            );
            // Changes received before a stop request are still reconciled.
            if !dirty.hosts.is_empty() {
                let hosts = dirty.hosts.clone();
                let reconciled = self.reconcile(&hosts);
                for host in &reconciled.hosts {
                    match report
                        .hosts
                        .iter_mut()
                        .find(|known| known.host == host.host)
                    {
                        Some(known) => *known = host.clone(),
                        None => report.hosts.push(host.clone()),
                    }
                }
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
                        if let Some(error) = classify(event, &mut dirty, &self.config, &self.homes)
                        {
                            self.shared.update(|state| state.last_error = Some(error));
                        }
                    }
                }
                drain(
                    &rx,
                    self.config.debounce,
                    &mut dirty,
                    &self.config,
                    &self.homes,
                    &self.shared,
                );
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
    /// after enumeration still produces an event.
    fn reconcile(&mut self, hosts: &[Host]) -> ImportReport {
        self.probe(ProbePoint::Reconciling(hosts));
        self.ensure_watches();
        self.scan(hosts, &mut |_| {})
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

    fn ensure_watches(&mut self) {
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
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
        for root in watch_roots(&self.config.home, &self.config.hosts) {
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
                Ok(()) => self.shared.update(|state| {
                    state.watched.insert(root.path.clone(), identity);
                    state.watch_installs += 1;
                }),
                Err(error) => {
                    let reason = format!("{} could not be watched: {error}", root.path.display());
                    self.shared.update(|state| {
                        state.freshness = Some(Freshness::Degraded {
                            reason: reason.clone(),
                        });
                        state.last_error = Some(reason);
                    });
                }
            }
        }
    }
}

struct WatchRoot {
    path: PathBuf,
    recursive: bool,
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
    fn add(roots: &mut Vec<WatchRoot>, home: &Path, candidates: &[PathBuf]) {
        for candidate in candidates {
            if candidate.is_dir() {
                if !roots.iter().any(|root| root.path == *candidate) {
                    roots.push(WatchRoot {
                        path: candidate.clone(),
                        recursive: true,
                    });
                }
                return;
            }
        }
        if !roots.iter().any(|root| root.path == home) {
            roots.push(WatchRoot {
                path: home.to_path_buf(),
                recursive: false,
            });
        }
    }
    let mut roots: Vec<WatchRoot> = Vec::new();
    for host in hosts {
        match host {
            Host::Claude => add(
                &mut roots,
                home,
                &[home.join(".claude/projects"), home.join(".claude")],
            ),
            Host::Codex => add(
                &mut roots,
                home,
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
                            add(&mut roots, home, &[root]);
                        }
                    }
                    if parent.is_dir() && !roots.iter().any(|root| root.path == parent) {
                        roots.push(WatchRoot {
                            path: parent,
                            recursive: false,
                        });
                    }
                } else {
                    add(&mut roots, home, &[parent]);
                }
            }
            Host::Other => {}
        }
    }
    roots
}

/// The host a changed path belongs to, by its position under the home (in
/// any spelling the platform may report it).
fn host_of(homes: &[PathBuf], path: &Path) -> Option<Host> {
    let relative = homes.iter().find_map(|home| path.strip_prefix(home).ok())?;
    match relative.components().next()?.as_os_str().to_str()? {
        ".claude" => Some(Host::Claude),
        ".codex" => Some(Host::Codex),
        ".cursor" => Some(Host::Cursor),
        _ => None,
    }
}

/// Note what an event touched. Kinds are not trusted: any path under a host
/// root marks that host, and a rescan request or watcher error marks every
/// host. Returns the watcher's error text, if it reported one.
fn classify(
    event: notify::Result<notify::Event>,
    dirty: &mut Dirty,
    config: &WatchConfig,
    homes: &[PathBuf],
) -> Option<String> {
    match event {
        Ok(event) => {
            if event.need_rescan() {
                dirty.rescan = true;
                dirty.mark_all(&config.hosts);
            }
            for path in &event.paths {
                if let Some(host) = host_of(homes, path)
                    && config.hosts.contains(&host)
                {
                    dirty.mark(host);
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
    shared: &Shared,
) {
    loop {
        match rx.recv_timeout(debounce) {
            Ok(Message::Fs(event)) => {
                if let Some(error) = classify(event, dirty, config, homes) {
                    shared.update(|state| state.last_error = Some(error));
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
                (home.join(".cursor"), false)
            ]
        );
    }
}
