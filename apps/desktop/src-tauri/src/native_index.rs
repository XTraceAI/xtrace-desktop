//! The native index the app keeps over local Claude, Codex and Cursor history.
//!
//! At startup the app resolves the reader hosts' interpreter, verifies the
//! bundled reader sources against the pin compiled in, opens its own
//! connection to the application database and starts the ingest crate's
//! tailer over the home directory: the initial scan, then live reconciliation
//! of changes. Every event the tailer reports becomes a typed status the
//! frontend can query and is published to it. Quitting cancels the scan in
//! progress, kills and reaps a reader still running, and waits a bounded time
//! for the worker, so a reader that never returns cannot hold the exit.
use crate::dto::{
    NativeFreshness, NativeHostState, NativeHostStatus, NativeIndexPhase, NativeIndexStatus,
    PythonRuntime, ReaderBundle,
};
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use xt_ingest::native::{
    HostReport, HostStatus, ImportReport, ProducerSource, SessionOutcome,
    readers_cli::{discover_python, parse_pin, verify_bundle},
    validate_index_destination,
    watch::{Freshness, ProbePoint, TailEvent, Tailer, WatchConfig},
};
use xt_store::{Host, Store};

/// The pin the bundled readers are verified against, compiled in so the app
/// needs no pin file, checkout or Git at runtime.
pub const PIN: &str = include_str!("../../../../.plugin-pin");
/// Where the bundled reader sources live below the app's resource directory.
pub const BUNDLE_RESOURCE: &str = "agent-plugins";
const HOSTS: [Host; 3] = [Host::Claude, Host::Codex, Host::Cursor];
/// Quiet period after the last filesystem event before a burst is reconciled.
const DEBOUNCE: Duration = Duration::from_millis(250);
/// How long quitting waits for the cancelled worker to end.
pub const SHUTDOWN_BOUND: Duration = Duration::from_secs(3);
/// Progress during the initial scan is published at most this often.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

pub struct NativeIndexOptions {
    /// The home whose `.claude`, `.codex`, `.cursor` and Cursor hook state are indexed.
    pub home: PathBuf,
    /// The application database the index is written to.
    pub db: PathBuf,
    /// The bundled reader sources, laid out as in the producer repository.
    pub bundle: PathBuf,
    /// An interpreter named by the user; otherwise one is discovered.
    pub python: Option<OsString>,
}

/// Receives every status change (the app emits it to the frontend).
pub type Publish = Arc<dyn Fn(&NativeIndexStatus) + Send + Sync>;

pub struct NativeIndex {
    tailer: Mutex<Option<Tailer>>,
    status: Arc<Mutex<NativeIndexStatus>>,
    publish: Publish,
}

impl NativeIndex {
    /// No index runs: the status says why.
    pub fn disabled(reason: impl Into<String>, publish: Publish) -> Self {
        let status = NativeIndexStatus {
            phase: NativeIndexPhase::Disabled {
                reason: reason.into(),
            },
            freshness: NativeFreshness::Unknown,
            python: PythonRuntime::Missing {
                reason: "not resolved: the index is disabled".into(),
            },
            readers: ReaderBundle::Unavailable {
                reason: "not verified: the index is disabled".into(),
            },
            hosts: Vec::new(),
            reconciles: 0,
            files_scanned: 0,
        };
        publish(&status);
        Self {
            tailer: Mutex::new(None),
            status: Arc::new(Mutex::new(status)),
            publish,
        }
    }

    /// Resolve the prerequisites, then start the tailer. A prerequisite the
    /// reader hosts need (the interpreter, the bundle) is reported in the
    /// status and by those hosts' scans while Claude indexing proceeds; a
    /// database the index must not use, or one that cannot be opened,
    /// disables the index with the reason.
    pub fn start(options: NativeIndexOptions, publish: Publish) -> Self {
        if let Err(reason) = validate_index_destination(&options.db, &options.home) {
            return Self::disabled(reason, publish);
        }
        let store = match Store::open(&options.db) {
            Ok(store) => store,
            Err(_) => return Self::disabled("the index database could not be opened", publish),
        };
        let python = match discover_python(options.python.as_deref()) {
            Ok(path) => PythonRuntime::Available {
                path: path.to_string_lossy().into_owned(),
            },
            Err(error) => PythonRuntime::Missing {
                reason: error.to_string(),
            },
        };
        let pin = parse_pin(PIN);
        let readers = match pin
            .as_ref()
            .map_err(ToString::to_string)
            .and_then(|pin| verify_bundle(pin, &options.bundle).map_err(|e| e.to_string()))
        {
            Ok(producer) => ReaderBundle::Verified {
                commit: producer.commit,
                plugin_version: producer.plugin_version,
            },
            Err(reason) => ReaderBundle::Unavailable { reason },
        };
        let status = Arc::new(Mutex::new(NativeIndexStatus {
            phase: NativeIndexPhase::Scanning,
            freshness: NativeFreshness::Unknown,
            python: python.clone(),
            readers,
            hosts: HOSTS.iter().map(|host| pending(*host)).collect(),
            reconciles: 0,
            files_scanned: 0,
        }));
        publish(&lock(&status));
        let producer = match pin {
            Ok(pin) => ProducerSource::Bundle {
                pin,
                root: options.bundle,
            },
            // An unparseable compiled pin cannot happen in a built app (a
            // test guards it); every reader scan then reports the mismatch.
            Err(_) => ProducerSource::Checkout {
                pin: PathBuf::from(".plugin-pin"),
                plugin_root: None,
            },
        };
        let probe = {
            let status = Arc::clone(&status);
            let publish = Arc::clone(&publish);
            let last = Mutex::new(Instant::now());
            Arc::new(move |point: ProbePoint<'_>| {
                if let ProbePoint::FileScanned(_) = point {
                    let mut current = lock(&status);
                    current.files_scanned = current.files_scanned.saturating_add(1);
                    let mut last = last.lock().unwrap_or_else(|p| p.into_inner());
                    if last.elapsed() >= PROGRESS_INTERVAL {
                        *last = Instant::now();
                        publish(&current);
                    }
                }
            }) as xt_ingest::native::watch::Probe
        };
        let sink = {
            let status = Arc::clone(&status);
            let publish = Arc::clone(&publish);
            Box::new(move |event: TailEvent| {
                let mut current = lock(&status);
                match event {
                    TailEvent::Ready(readiness) => {
                        current.phase = NativeIndexPhase::Ready;
                        current.freshness = freshness(&readiness.freshness);
                        absorb(&mut current, &readiness.report);
                    }
                    TailEvent::Reconciled {
                        freshness: fresh,
                        report,
                        ..
                    } => {
                        current.freshness = freshness(&fresh);
                        absorb(&mut current, &report);
                    }
                    TailEvent::Stopped { freshness: fresh } => {
                        current.phase = NativeIndexPhase::Stopped;
                        current.freshness = freshness(&fresh);
                    }
                }
                publish(&current);
            }) as Box<dyn Fn(TailEvent) + Send>
        };
        let tailer = Tailer::start(
            store,
            WatchConfig {
                home: options.home,
                hosts: HOSTS.to_vec(),
                producer,
                // An interpreter the user named is used as named, qualified
                // or not; otherwise each reader scan discovers again, so one
                // installed later is found without a restart.
                python: options.python,
                debounce: DEBOUNCE,
                probe: Some(probe),
            },
            sink,
        );
        Self {
            tailer: Mutex::new(Some(tailer)),
            status,
            publish,
        }
    }

    pub fn status(&self) -> NativeIndexStatus {
        lock(&self.status).clone()
    }

    /// Cancel the scan in progress, kill and reap a reader still running, and
    /// wait at most `SHUTDOWN_BOUND` for the worker. Returns false if the
    /// worker had not ended within the bound (it ends with the process).
    pub fn shutdown(&self) -> bool {
        let tailer = self
            .tailer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let Some(tailer) = tailer else {
            return true;
        };
        let ended = tailer.shutdown(SHUTDOWN_BOUND);
        let mut current = lock(&self.status);
        if current.phase != NativeIndexPhase::Stopped {
            current.phase = NativeIndexPhase::Stopped;
            (self.publish)(&current);
        }
        ended
    }
}

fn lock(status: &Mutex<NativeIndexStatus>) -> std::sync::MutexGuard<'_, NativeIndexStatus> {
    status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn pending(host: Host) -> NativeHostStatus {
    NativeHostStatus {
        host: host.as_str().to_owned(),
        state: NativeHostState::Pending,
        detail: None,
        sessions_imported: 0,
        sessions_partial: 0,
        sessions_skipped: 0,
        records_new: 0,
        records_enriched: 0,
        diagnostics: 0,
    }
}

fn freshness(value: &Freshness) -> NativeFreshness {
    match value {
        Freshness::Live => NativeFreshness::Live,
        Freshness::Degraded { reason } => NativeFreshness::Degraded {
            reason: reason.clone(),
        },
    }
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn host_status(report: &HostReport) -> NativeHostStatus {
    let mut status = pending(report.host);
    status.state = match report.status {
        HostStatus::Complete => NativeHostState::Complete,
        HostStatus::Incomplete => NativeHostState::Incomplete,
        HostStatus::MissingSource => NativeHostState::MissingSource,
        HostStatus::MissingRuntime => NativeHostState::MissingRuntime,
        HostStatus::PinMismatch => NativeHostState::PinMismatch,
        HostStatus::ReaderFailed => NativeHostState::ReaderFailed,
        HostStatus::Cancelled => NativeHostState::Cancelled,
    };
    status.detail = report.detail.clone();
    status.diagnostics = count(report.diagnostics.len());
    let (mut new, mut enriched) = (0usize, 0usize);
    for session in &report.sessions {
        match &session.outcome {
            SessionOutcome::Imported {
                records_new,
                records_enriched,
            } => {
                status.sessions_imported += 1;
                new += records_new;
                enriched += records_enriched;
            }
            SessionOutcome::Partial {
                records_new,
                records_enriched,
                ..
            } => {
                status.sessions_partial += 1;
                new += records_new;
                enriched += records_enriched;
            }
            SessionOutcome::Skipped { .. } => status.sessions_skipped += 1,
        }
    }
    status.records_new = count(new);
    status.records_enriched = count(enriched);
    status
}

/// A scan's report replaces the status of every host it covered; a host it
/// did not touch keeps its last scan.
fn absorb(status: &mut NativeIndexStatus, report: &ImportReport) {
    status.reconciles = status.reconciles.saturating_add(1);
    for host in &report.hosts {
        let mapped = host_status(host);
        match status
            .hosts
            .iter_mut()
            .find(|known| known.host == mapped.host)
        {
            Some(known) => *known = mapped,
            None => status.hosts.push(mapped),
        }
    }
}
