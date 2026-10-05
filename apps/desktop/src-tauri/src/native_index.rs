//! The native index the app keeps over local Claude, Codex and Cursor history.
//!
//! At startup the app resolves the reader hosts' interpreter, verifies the
//! bundled reader sources against the pin compiled in, opens its own
//! connection to the application database and starts the ingest crate's
//! tailer over the home directory: the initial scan, then live reconciliation
//! of changes. Every event the tailer reports becomes a typed status the
//! frontend can query and is published to it, except a committed change to
//! sub-session relations between reconciliations, which is published on its
//! own and leaves the status as it was. Quitting cancels the scan in
//! progress, kills and reaps a reader still running, and waits a bounded time
//! for the worker, so a reader that never returns cannot hold the exit.
use crate::dto::{
    NativeFreshness, NativeHostState, NativeHostStatus, NativeIndexPhase, NativeIndexStatus,
    NativeSkipReason, NativeSkippedConversation, PythonRuntime, ReaderBundle,
    ReaderUnavailableCause,
};
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use xt_ingest::native::{
    CancelToken, HostReport, HostStatus, ImportReport, ProducerSource, SessionOutcome,
    SessionResult,
    readers_cli::{PinnedProducer, ReaderError, discover_python, parse_pin, verify_bundle},
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
const SKIPPED_CONVERSATION_LIMIT: usize = 100;

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

/// Called once after each background pass that committed a change to stored
/// sub-session relations (the app emits a data event that re-reads the
/// Sessions and Dashboard views). It carries nothing, and the status, its
/// freshness and its reconciliation count stay as they were.
pub type PublishCreations = Arc<dyn Fn() + Send + Sync>;

/// What one transcript open needs to read a Codex or Cursor session: the
/// bundled readers, verified against the compiled pin **at each open**, and an
/// interpreter resolved under that open's own cancel token. Nothing is cached
/// between opens, so a bundle edited after startup is refused and an
/// interpreter installed later is found, as the index's own scans do.
#[derive(Clone, Debug)]
pub struct DetailReaders {
    source: DetailSource,
    /// An interpreter the user named, probed as named; otherwise discovered.
    python: Option<OsString>,
}

#[derive(Clone, Debug)]
enum DetailSource {
    /// No local history is read: fixture startup, or a disabled index.
    Disabled,
    /// The bundled readers, laid out as in the producer repository.
    Bundle(PathBuf),
}

/// A verified producer and a qualified interpreter for one open.
pub struct ResolvedReaders {
    pub python: OsString,
    pub producer: PinnedProducer,
}

/// Why an open could not be given a reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadersUnready {
    /// The open was cancelled while its interpreter was being resolved.
    Cancelled,
    Unavailable(ReaderUnavailableCause),
}

impl DetailReaders {
    /// No reader can be given: every Codex or Cursor open says so.
    pub fn disabled() -> Self {
        Self {
            source: DetailSource::Disabled,
            python: None,
        }
    }

    /// The bundled readers at `bundle`, with the interpreter the user named.
    pub fn bundle(bundle: PathBuf, python: Option<OsString>) -> Self {
        Self {
            source: DetailSource::Bundle(bundle),
            python,
        }
    }

    /// Verify the bundle and resolve the interpreter for one open. The bundle
    /// is checked first, since it costs no process; the interpreter probe is
    /// bounded and is killed by a cancel of the open.
    pub fn resolve(&self, cancel: &CancelToken) -> Result<ResolvedReaders, ReadersUnready> {
        let DetailSource::Bundle(bundle) = &self.source else {
            return Err(ReadersUnready::Unavailable(ReaderUnavailableCause::Index));
        };
        if cancel.is_cancelled() {
            return Err(ReadersUnready::Cancelled);
        }
        let producer = parse_pin(PIN)
            .and_then(|pin| verify_bundle(&pin, bundle))
            .map_err(|_| ReadersUnready::Unavailable(ReaderUnavailableCause::Readers))?;
        let python = match discover_python(self.python.as_deref(), Some(cancel)) {
            Ok(python) => python,
            Err(ReaderError::Cancelled) => return Err(ReadersUnready::Cancelled),
            Err(_) => {
                return Err(ReadersUnready::Unavailable(
                    ReaderUnavailableCause::Interpreter,
                ));
            }
        };
        Ok(ResolvedReaders { python, producer })
    }
}

pub struct NativeIndex {
    tailer: Mutex<Option<Tailer>>,
    status: Arc<Mutex<NativeIndexStatus>>,
    publish: Publish,
    detail: DetailReaders,
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
            detail: DetailReaders::disabled(),
        }
    }

    /// Resolve the prerequisites, then start the tailer. A prerequisite the
    /// reader hosts need (the interpreter, the bundle) is reported in the
    /// status and by those hosts' scans while Claude indexing proceeds; a
    /// database the index must not use, or one that cannot be opened,
    /// disables the index with the reason.
    pub fn start(options: NativeIndexOptions, publish: Publish) -> Self {
        Self::start_with_creations(options, publish, Arc::new(|| {}))
    }

    /// [`Self::start`], also calling `creations` after every background pass
    /// that committed a change to stored sub-session relations.
    pub fn start_with_creations(
        options: NativeIndexOptions,
        publish: Publish,
        creations: PublishCreations,
    ) -> Self {
        if let Err(reason) = validate_index_destination(&options.db, &options.home) {
            return Self::disabled(reason, publish);
        }
        let store = match Store::open(&options.db) {
            Ok(store) => store,
            Err(_) => return Self::disabled("the index database could not be opened", publish),
        };
        let detail = DetailReaders::bundle(options.bundle.clone(), options.python.clone());
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
            python: PythonRuntime::Resolving,
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
                if let TailEvent::SessionCreationsChanged { .. } = event {
                    creations();
                    return;
                }
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
                    TailEvent::SessionCreationsChanged { .. } => return,
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
                python: options.python.clone(),
                debounce: DEBOUNCE,
                spawn_limits: xt_ingest::native::session_creation::spawn_limits(),
                probe: Some(probe),
            },
            sink,
        );
        // Discovery for the status runs beside the initial scan, never on
        // the caller's thread: every probe is bounded, and registered with
        // the tailer's token so shutdown kills one that is still running.
        {
            let status = Arc::clone(&status);
            let publish = Arc::clone(&publish);
            let cancel = tailer.cancel_token();
            let named = options.python;
            std::thread::Builder::new()
                .name("xtrace-native-python".into())
                .spawn(move || {
                    let resolved = match discover_python(named.as_deref(), Some(&cancel)) {
                        Ok(path) => PythonRuntime::Available {
                            path: path.to_string_lossy().into_owned(),
                        },
                        Err(error) => PythonRuntime::Missing {
                            reason: error.to_string(),
                        },
                    };
                    let mut current = lock(&status);
                    current.python = resolved;
                    publish(&current);
                })
                .expect("spawn the interpreter discovery thread");
        }
        Self {
            tailer: Mutex::new(Some(tailer)),
            status,
            publish,
            detail,
        }
    }

    pub fn status(&self) -> NativeIndexStatus {
        lock(&self.status).clone()
    }

    /// The reader transcript opens use for Codex and Cursor sessions.
    pub fn detail_readers(&self) -> &DetailReaders {
        &self.detail
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
        skipped_conversations: Vec::new(),
        skipped_conversations_omitted: 0,
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
            SessionOutcome::Skipped { reason } => {
                status.sessions_skipped += 1;
                if status.skipped_conversations.len() < SKIPPED_CONVERSATION_LIMIT {
                    status
                        .skipped_conversations
                        .push(NativeSkippedConversation {
                            conversation_id: skipped_conversation_id(report.host, session),
                            reason: skip_reason(reason),
                        });
                } else {
                    status.skipped_conversations_omitted += 1;
                }
            }
        }
    }
    status.records_new = count(new);
    status.records_enriched = count(enriched);
    status
}

/// Accept only the hosts' UUID conversation forms, agreeing with the native ID.
/// An arbitrary source label can be a path, credential or text even if bounded.
fn skipped_conversation_id(host: Host, session: &SessionResult) -> Option<String> {
    let native = session.native_session_id.as_deref()?;
    if native.len() != 36
        || !native.bytes().enumerate().all(|(i, byte)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
        || host == Host::Other
    {
        return None;
    }
    let expected = xt_ingest::native::stream::expected_conversation_id(host, native);
    session
        .conversation_id
        .as_ref()
        .filter(|id| **id == expected)
        .cloned()
}

/// Match fixed producer/writer wording; discard all error suffixes. Unknown
/// wording stays unknown instead of guessing from arbitrary source text.
fn skip_reason(reason: &str) -> NativeSkipReason {
    use NativeSkipReason::*;
    if reason.starts_with("file could not be read: ") {
        Unreadable
    } else if [
        "transcript is not UTF-8 (stream line ",
        "canonical record does not match the shared stream contract (stream line ",
    ]
    .iter()
    .any(|prefix| reason.starts_with(prefix))
    {
        InvalidTranscript
    } else if [
        "session identity is blank",
        "discovered identity conflicts with the index: ",
        "completed header identity conflicts with the index: ",
        "native record identity labels disagree (stream line ",
        "structural summary identity labels disagree (stream line ",
        "PR witness identity labels disagree (stream line ",
    ]
    .iter()
    .any(|prefix| reason.starts_with(prefix))
    {
        IdentityConflict
    } else if [
        "record has an unusable surface (stream line ",
        "dropped record has an unusable surface (stream line ",
        "record surface disagrees with the file's surface (stream line ",
        "structural summary surface disagrees with the file's surface (stream line ",
        "PR witness surface disagrees with the file's surface (stream line ",
    ]
    .iter()
    .any(|prefix| reason.starts_with(prefix))
    {
        InvalidSurface
    } else if [
        "session header does not match the shared stream contract (stream line ",
        "session header names another host (stream line ",
        "session header lacks an identity (stream line ",
        "session header conversation ID does not derive from its native ID (stream line ",
        "session header carries an empty label (stream line ",
        "session header lacks a source path (stream line ",
        "session header start is not RFC3339 (stream line ",
        "session header clock is not a representable instant (stream line ",
    ]
    .iter()
    .any(|prefix| reason.starts_with(prefix))
    {
        InvalidHeader
    } else if [
        "checkpoint could not be read: ",
        "checkpoint could not be refreshed: ",
        "checkpoint could not be recorded after the rows: ",
        "cursor could not be recorded after ",
    ]
    .iter()
    .any(|prefix| reason.starts_with(prefix))
    {
        CheckpointFailed
    } else if reason.starts_with("session could not be written after ") {
        WriteFailed
    } else if reason.starts_with("reader ended before completing this session; ") {
        ReaderIncomplete
    } else {
        Unknown
    }
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

#[cfg(test)]
mod skipped_details_tests {
    use super::*;
    use xt_ingest::native::{
        ImportRequest, import_native, import_reader_lines, readers_cli::ReaderOutcome,
    };

    const ID: &str = "00000000-0000-4000-8000-000000000001";

    fn skipped(host: Host, native: &str, reason: &str) -> SessionResult {
        SessionResult {
            native_session_id: Some(native.into()),
            conversation_id: Some(xt_ingest::native::stream::expected_conversation_id(
                host, native,
            )),
            source_surface: Some("private source label".into()),
            path: Some("/private/synthetic/credential.jsonl".into()),
            outcome: SessionOutcome::Skipped {
                reason: reason.into(),
            },
        }
    }

    fn report(host: Host, sessions: Vec<SessionResult>) -> HostReport {
        HostReport {
            host,
            status: HostStatus::Incomplete,
            detail: None,
            diagnostics: vec![],
            sessions,
            origin: None,
        }
    }

    #[test]
    fn only_validated_conversation_ids_and_fixed_reasons_cross_ipc() {
        for host in [Host::Claude, Host::Codex, Host::Cursor] {
            let valid = skipped(
                host,
                ID,
                "file could not be read: PermissionDenied: /private/synthetic/credential.jsonl",
            );
            let mut sessions = vec![valid.clone()];
            for unsafe_id in [
                "/private/synthetic/credential.jsonl",
                "Bearer synthetic-token",
                "<script>synthetic</script>",
                "\nsecret",
                "00000000-0000-4000-8000-00000000000g",
            ] {
                sessions.push(skipped(host, unsafe_id, "secret synthetic transcript"));
            }
            sessions.push(skipped(
                host,
                &"a".repeat(10_000),
                "secret synthetic transcript",
            ));
            let mut mismatch = valid.clone();
            mismatch.conversation_id = Some("00000000-0000-4000-8000-000000000002".into());
            sessions.push(mismatch);
            let mut missing = valid;
            missing.native_session_id = None;
            sessions.push(missing);
            let status = host_status(&report(host, sessions));
            assert_eq!(status.sessions_skipped, 9);
            assert_eq!(
                status.skipped_conversations[0].conversation_id,
                Some(xt_ingest::native::stream::expected_conversation_id(
                    host, ID
                ))
            );
            assert_eq!(
                status.skipped_conversations[0].reason,
                NativeSkipReason::Unreadable
            );
            assert!(
                status.skipped_conversations[1..]
                    .iter()
                    .all(|s| s.conversation_id.is_none())
            );
            assert_eq!(
                status.skipped_conversations[1].reason,
                NativeSkipReason::Unknown
            );
            let json = serde_json::to_string(&status).unwrap();
            for withheld in [
                "private",
                "credential",
                "Bearer",
                "script",
                "secret",
                "synthetic",
                "PermissionDenied",
            ] {
                assert!(!json.contains(withheld), "{json}");
            }
        }
        assert!(
            host_status(&report(
                Host::Other,
                vec![skipped(Host::Other, ID, "unknown")]
            ))
            .skipped_conversations[0]
                .conversation_id
                .is_none()
        );
    }

    #[test]
    fn bounds_and_replacement_reports_leave_counts_and_retained_facts_truthful() {
        let mut sessions = vec![skipped(Host::Claude, ID, "unknown"); 103];
        let mut imported = skipped(Host::Claude, ID, "unused");
        imported.outcome = SessionOutcome::Imported {
            records_new: 7,
            records_enriched: 2,
        };
        sessions.push(imported);
        let mut partial = skipped(Host::Claude, ID, "unused");
        partial.outcome = SessionOutcome::Partial {
            records_new: 3,
            records_enriched: 1,
            records_dropped: 1,
            rejections: vec!["private raw rejection".into()],
        };
        sessions.push(partial);
        let mut cancelled = report(Host::Claude, sessions);
        cancelled.status = HostStatus::Cancelled;
        let mapped = host_status(&cancelled);
        assert_eq!(mapped.state, NativeHostState::Cancelled);
        assert_eq!(
            (
                mapped.sessions_imported,
                mapped.sessions_partial,
                mapped.sessions_skipped,
                mapped.records_new,
                mapped.records_enriched
            ),
            (1, 1, 103, 10, 3)
        );
        assert_eq!(mapped.skipped_conversations.len(), 100);
        assert_eq!(mapped.skipped_conversations_omitted, 3);
        let mut status = NativeIndex::disabled("test", Arc::new(|_| {})).status();
        let publish_report = |status: &mut NativeIndexStatus, host| {
            absorb(status, &ImportReport { hosts: vec![host] })
        };
        publish_report(&mut status, cancelled);
        // An untouched host retains exactly the scan facts already reported.
        publish_report(&mut status, report(Host::Codex, vec![]));
        assert_eq!(status.hosts[0], mapped);
        for state in [
            HostStatus::Complete,
            HostStatus::Cancelled,
            HostStatus::MissingSource,
        ] {
            let mut empty = report(Host::Claude, vec![]);
            empty.status = state;
            publish_report(&mut status, empty);
            assert!(status.hosts[0].skipped_conversations.is_empty());
            assert_eq!(status.hosts[0].skipped_conversations_omitted, 0);
            assert_eq!(status.hosts[0].sessions_skipped, 0);
        }
    }

    #[test]
    fn real_claude_skip_sources_map_and_a_recovered_scan_clears_them() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let project = home.join(".claude/projects/-synthetic");
        std::fs::create_dir_all(&project).unwrap();
        let ids = [
            ID,
            "00000000-0000-4000-8000-000000000002",
            "00000000-0000-4000-8000-000000000003",
        ];
        let identity_conflict = serde_json::json!({"type":"user", "uuid":"11111111-1111-4111-8111-111111111111", "native_session_id":ids[0], "sessionId":ids[2], "timestamp":"2026-09-07T12:00:00Z", "message":{"role":"user","content":"synthetic"}}).to_string() + "\n";
        for (id, bytes) in ids.into_iter().zip([
            b"{broken synthetic json}\n".as_slice(),
            b"\xff\n",
            identity_conflict.as_bytes(),
        ]) {
            std::fs::write(project.join(format!("{id}.jsonl")), bytes).unwrap();
        }
        let mut store = Store::open_in_memory().unwrap();
        let producer = ProducerSource::Checkout {
            pin: temp.path().join("unused"),
            plugin_root: None,
        };
        let request = ImportRequest {
            home: &home,
            hosts: &[Host::Claude],
            producer: &producer,
            python: None,
            observed_at: 1_788_782_400_000,
            cancel: None,
        };
        let first = import_native(&mut store, &request);
        let status = host_status(&first.hosts[0]);
        assert_eq!(
            (
                status.sessions_imported,
                status.sessions_partial,
                status.sessions_skipped
            ),
            (0, 0, 3)
        );
        for (id, reason) in ids.into_iter().zip([
            NativeSkipReason::InvalidTranscript,
            NativeSkipReason::InvalidTranscript,
            NativeSkipReason::IdentityConflict,
        ]) {
            let detail = status
                .skipped_conversations
                .iter()
                .find(|s| s.conversation_id.as_deref() == Some(id))
                .unwrap();
            assert_eq!(detail.reason, reason);
            std::fs::write(project.join(format!("{id}.jsonl")), b"\n").unwrap();
        }
        let recovered = host_status(&import_native(&mut store, &request).hosts[0]);
        assert_eq!(
            (
                recovered.sessions_imported,
                recovered.sessions_partial,
                recovered.sessions_skipped
            ),
            (3, 0, 0)
        );
        assert!(recovered.skipped_conversations.is_empty());
    }

    #[test]
    fn real_reader_header_failure_maps_without_publishing_the_header() {
        let mut store = Store::open_in_memory().unwrap();
        let lines = [serde_json::json!({"type":"session", "host":"codex", "native_session_id":ID, "conversation_id":format!("codex-{ID}"), "path":"/private/synthetic/credential.jsonl"}).to_string()];
        let report = import_reader_lines(
            &mut store,
            Host::Codex,
            "test".into(),
            lines.into_iter().map(Ok),
            1_788_782_400_000,
            || {
                Ok(ReaderOutcome {
                    diagnostics: vec![],
                    complete: true,
                })
            },
        );
        let status = host_status(&report);
        assert_eq!(status.sessions_skipped, 1);
        assert_eq!(
            status.skipped_conversations[0].reason,
            NativeSkipReason::InvalidHeader
        );
        assert!(
            !serde_json::to_string(&status)
                .unwrap()
                .contains("credential")
        );
    }
}
