//! Initial import of native session history into the local store.
//!
//! Codex and Cursor history is read by the pinned shared readers from
//! `.plugin-pin`; Claude history is read directly from its canonical JSONL.
//! Every host goes through one path: the shared stream (header + records) is
//! split into sessions and each session is written through the transactional
//! canonical writer under the persisted content policy, which defaults to
//! metadata only. Native files are never written, no plugin needs to be
//! installed and nothing contacts a network. Absent sources, a missing Python
//! runtime, a producer that is not the pinned one, reader failures and
//! malformed output are reported explicitly per host and per session, while
//! every readable session still imports.

pub mod checkpoint;
pub mod claude_fs;
pub mod readers_cli;
pub mod stream;
pub mod watch;

use crate::writer::{MAX_BATCH_RECORDS, WriteBatch, write_batch};
pub use readers_cli::{CancelToken, ProducerSource};
use readers_cli::{ReaderDiagnostic, ReaderError};
use serde::Serialize;
use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
};
use xt_store::{
    Host, SessionSource, Store,
    batch::{NativeCheckpoint, RecordDisposition, SourceCursor},
    ingest::DiscoveredSession,
};

/// Whether proven checkpoints may shorten a scan of Claude transcripts. Reader
/// hosts (Codex, Cursor) are read whole through the pinned producer in both
/// modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanMode {
    /// Resume behind checkpoints whose generation is proven again; read
    /// everything else. Unchanged transcripts cost a stat and a short read.
    Resume,
    /// Read every transcript from its start; checkpoints are still recorded.
    Replay,
}

pub struct ImportRequest<'a> {
    /// The home directory whose `.claude`, `.codex` and `.cursor` trees are
    /// read. A relative path is anchored to the current directory once, up
    /// front, so the readers (which run inside it) and every reported path and
    /// cursor key see one absolute spelling.
    pub home: &'a Path,
    pub hosts: &'a [Host],
    /// Where the pinned producer's sources are; used for Codex and Cursor.
    pub producer: &'a ProducerSource,
    pub python: Option<&'a OsStr>,
    /// UTC milliseconds recorded as the observation time of this import.
    pub observed_at: i64,
    /// Cancels the scan: a reader running is killed, a host not yet scanned or
    /// a Claude file not yet read is left for the next scan, and each is
    /// reported as cancelled. What was committed before stays.
    pub cancel: Option<&'a CancelToken>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostStatus {
    /// Every discovered session of this host was read and imported.
    Complete,
    /// The producer or a session reported incomplete coverage; readable
    /// sessions are imported and the gaps are listed.
    Incomplete,
    MissingSource,
    MissingRuntime,
    PinMismatch,
    ReaderFailed,
    /// The scan was cancelled before this host was read completely: the
    /// sessions listed were imported, the rest wait for the next scan.
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum SessionOutcome {
    /// Every record of the session was accepted.
    Imported {
        records_new: usize,
        records_enriched: usize,
    },
    /// The session's identity and accepted records are stored, but some records
    /// could not be: they lacked a UUID, or the writer rejected them (a UUID
    /// owned by another session, a type conflict). Coverage is incomplete.
    Partial {
        records_new: usize,
        records_enriched: usize,
        records_dropped: usize,
        rejections: Vec<String>,
    },
    Skipped {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionResult {
    pub native_session_id: Option<String>,
    pub conversation_id: Option<String>,
    pub source_surface: Option<String>,
    /// Local source locator; index metadata only, never a cloud or telemetry payload.
    pub path: Option<String>,
    #[serde(flatten)]
    pub outcome: SessionOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HostReport {
    pub host: Host,
    pub status: HostStatus,
    pub detail: Option<String>,
    pub diagnostics: Vec<ReaderDiagnostic>,
    pub sessions: Vec<SessionResult>,
}

impl HostReport {
    fn unavailable(host: Host, status: HostStatus, detail: impl Into<String>) -> Self {
        Self {
            host,
            status,
            detail: Some(detail.into()),
            diagnostics: Vec::new(),
            sessions: Vec::new(),
        }
    }
}

pub(crate) fn all_imported(sessions: &[SessionResult]) -> bool {
    sessions
        .iter()
        .all(|session| matches!(session.outcome, SessionOutcome::Imported { .. }))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub hosts: Vec<HostReport>,
}

impl ImportReport {
    /// True only when every requested host imported every record of every session.
    pub fn complete(&self) -> bool {
        self.hosts
            .iter()
            .all(|host| host.status == HostStatus::Complete)
    }
}

/// Import every requested host, resuming behind proven checkpoints. Failures
/// are reported, never raised: one host's missing runtime does not stop
/// another host's import.
pub fn import_native(store: &mut Store, request: &ImportRequest<'_>) -> ImportReport {
    scan_native(store, request, ScanMode::Resume)
}

/// Import every requested host in the given mode.
pub fn scan_native(store: &mut Store, request: &ImportRequest<'_>, mode: ScanMode) -> ImportReport {
    scan_native_observed(store, request, mode, &mut |_| {})
}

/// `scan_native` with an observer called after each Claude file is done, so a
/// caller can act between files (the watcher's startup race is tested this way).
pub fn scan_native_observed(
    store: &mut Store,
    request: &ImportRequest<'_>,
    mode: ScanMode,
    observer: &mut dyn FnMut(&Path),
) -> ImportReport {
    let home = match anchor(request.home) {
        Ok(home) => home,
        Err(error) => {
            let detail = format!("home cannot be anchored to the current directory: {error}");
            return ImportReport {
                hosts: request
                    .hosts
                    .iter()
                    .map(|host| HostReport::unavailable(*host, HostStatus::MissingSource, &detail))
                    .collect(),
            };
        }
    };
    let request = &ImportRequest {
        home: &home,
        hosts: request.hosts,
        producer: request.producer,
        python: request.python,
        observed_at: request.observed_at,
        cancel: request.cancel,
    };
    let hosts = request
        .hosts
        .iter()
        .map(|host| match host {
            _ if cancelled(request) => HostReport::unavailable(
                *host,
                HostStatus::Cancelled,
                "scan cancelled before this host was read",
            ),
            Host::Claude => import_claude(store, request, mode, observer),
            Host::Codex | Host::Cursor => import_reader_host(store, request, *host),
            Host::Other => HostReport::unavailable(
                Host::Other,
                HostStatus::MissingSource,
                "no native reader exists for an unknown host",
            ),
        })
        .collect();
    ImportReport { hosts }
}

fn cancelled(request: &ImportRequest<'_>) -> bool {
    request.cancel.is_some_and(CancelToken::is_cancelled)
}

/// Refuse an index destination inside the native sources it would index, or
/// aliased to them, before SQLite can create the database or its sidecars
/// there: the index's own writes must never read as source changes, and an
/// alternate name (an existing hard link) must not alias native history.
/// Source entries the process cannot read are passed over, never a reason to
/// refuse: the scan reports them as its own diagnostics.
pub fn validate_index_destination(db: &Path, home: &Path) -> Result<(), &'static str> {
    fn resolved(path: &Path) -> std::io::Result<PathBuf> {
        match path.canonicalize() {
            Ok(path) => Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if std::fs::symlink_metadata(path).is_ok() {
                    return Err(error);
                }
                let parent = path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                Ok(resolved(parent)?.join(path.file_name().ok_or(error)?))
            }
            Err(error) => Err(error),
        }
    }
    // An entry this process cannot read cannot hold an alias it would follow
    // either (the importers diagnose what they cannot list), so such entries
    // are passed over: a source-access problem is the scan's to report, not a
    // reason to refuse the destination. An alias that is found is refused.
    fn reject_reverse_aliases(root: &Path, destinations: &[PathBuf]) -> Result<(), &'static str> {
        let metadata = match std::fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                return Ok(());
            }
            Err(_) => return Err("Cannot inspect native source aliases"),
        };
        if metadata.file_type().is_symlink() {
            let target =
                std::fs::read_link(root).map_err(|_| "Cannot inspect native source alias")?;
            let target = resolved(&root.parent().unwrap_or(Path::new(".")).join(target))
                .map_err(|_| "Cannot resolve native source alias")?;
            if destinations.iter().any(|path| path.starts_with(&target)) {
                return Err("Native source alias points at index destination");
            }
        } else if metadata.is_dir() {
            let entries = match std::fs::read_dir(root) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                    return Ok(());
                }
                Err(_) => return Err("Cannot inspect native source aliases"),
            };
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                        continue;
                    }
                    Err(_) => return Err("Cannot inspect native source entry"),
                };
                reject_reverse_aliases(&entry.path(), destinations)?;
            }
        }
        Ok(())
    }
    let mut destinations = Vec::new();
    // The host history directories, and the hook state area the Cursor watch
    // covers: an index there would feed its own writes back as changes. A
    // root that cannot be resolved (a dangling alias, a parent this process
    // cannot traverse) is compared as spelled: nothing can be created inside
    // such a root, and a source-access problem is the scan's to report, not
    // a reason to refuse the destination.
    let home_spelled = resolved(home).unwrap_or_else(|_| home.to_path_buf());
    let roots = [".claude", ".codex", ".cursor", ".config/memhub-plugin"]
        .map(|name| resolved(&home.join(name)).unwrap_or_else(|_| home_spelled.join(name)));
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = db.as_os_str().to_os_string();
        name.push(suffix);
        let path = PathBuf::from(name);
        let target = resolved(&path).map_err(|_| "Cannot verify index destination")?;
        destinations.push(target.clone());
        if roots.iter().any(|root| target.starts_with(root)) {
            return Err("Index database must be outside native history directories");
        }
        #[cfg(unix)]
        if let Ok(metadata) = std::fs::metadata(&path) {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() > 1 {
                return Err("Index database or sidecar has multiple hard links");
            }
        }
    }
    for source in [
        ".claude/projects",
        ".codex/sessions",
        ".cursor/chats",
        ".cursor/projects",
        ".config/memhub-plugin/cursorflush",
    ] {
        reject_reverse_aliases(&home.join(source), &destinations)?;
    }
    Ok(())
}

/// The readers change into the home and receive it as `HOME`, so a relative
/// spelling would resolve beneath itself there; it is made absolute here, in
/// the caller's directory, without following symlinks.
fn anchor(home: &Path) -> std::io::Result<PathBuf> {
    if home.is_absolute() {
        return Ok(home.to_path_buf());
    }
    let mut anchored = std::env::current_dir()?;
    anchored.extend(
        home.components()
            .filter(|part| !matches!(part, Component::CurDir)),
    );
    Ok(anchored)
}

fn import_claude(
    store: &mut Store,
    request: &ImportRequest<'_>,
    mode: ScanMode,
    observer: &mut dyn FnMut(&Path),
) -> HostReport {
    let projects = request.home.join(".claude").join("projects");
    for root in [request.home.join(".claude"), projects.clone()] {
        match std::fs::symlink_metadata(&root) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return HostReport {
                    host: Host::Claude,
                    status: HostStatus::Incomplete,
                    detail: Some("Claude source root is an alias; source was not traversed".into()),
                    diagnostics: vec![ReaderDiagnostic {
                        code: "discovery_incomplete".into(),
                        path: Some(root.to_string_lossy().into_owned()),
                    }],
                    sessions: vec![],
                };
            }
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return HostReport::unavailable(
                    Host::Claude,
                    HostStatus::ReaderFailed,
                    "Claude source root is not a directory",
                );
            }
            Err(error) => {
                return HostReport::unavailable(
                    Host::Claude,
                    if error.kind() == std::io::ErrorKind::NotFound {
                        HostStatus::MissingSource
                    } else {
                        HostStatus::ReaderFailed
                    },
                    format!("Claude source root unavailable: {}", error.kind()),
                );
            }
        }
    }
    let (files, mut diagnostics) = match claude_fs::enumerate(&projects) {
        Ok(found) => found,
        Err(error) => {
            return HostReport::unavailable(
                Host::Claude,
                HostStatus::ReaderFailed,
                format!(
                    "~/.claude/projects could not be enumerated: {}",
                    error.kind()
                ),
            );
        }
    };
    let mut sessions = Vec::new();
    let mut interrupted = false;
    for file in files {
        if cancelled(request) {
            interrupted = true;
            break;
        }
        match claude_fs::import_file(store, &file, request.observed_at, mode) {
            Ok(result) => sessions.push(result),
            Err(error) => {
                diagnostics.push(ReaderDiagnostic {
                    code: "session_unreadable".into(),
                    path: Some(file.path.to_string_lossy().into_owned()),
                });
                sessions.push(SessionResult {
                    native_session_id: Some(file.session_id.clone()),
                    conversation_id: Some(file.session_id.clone()),
                    source_surface: None,
                    path: Some(file.path.to_string_lossy().into_owned()),
                    outcome: SessionOutcome::Skipped {
                        reason: format!("file could not be read: {}", error.kind()),
                    },
                });
            }
        }
        observer(&file.path);
    }
    let (status, detail) = if interrupted {
        (
            HostStatus::Cancelled,
            Some("scan cancelled before every transcript was read".to_owned()),
        )
    } else if diagnostics.is_empty() && all_imported(&sessions) {
        (HostStatus::Complete, None)
    } else {
        (HostStatus::Incomplete, None)
    };
    HostReport {
        host: Host::Claude,
        status,
        detail,
        diagnostics,
        sessions,
    }
}

fn reader_sources_present(home: &Path, host: Host) -> std::io::Result<bool> {
    let roots: &[&str] = match host {
        Host::Codex => &[".codex/sessions"],
        Host::Cursor => &[".cursor/chats", ".cursor/projects"],
        _ => &[],
    };
    let mut error = None;
    for root in roots {
        match std::fs::metadata(home.join(root)) {
            Ok(meta) if meta.is_dir() => return Ok(true),
            Ok(_) => error = Some(std::io::Error::other("source root is not a directory")),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => error = Some(err),
        }
    }
    match error {
        Some(error) => Err(error),
        None => Ok(false),
    }
}

/// Read one reader host whole through the pinned producer. Every scan reads
/// every session; records dedupe by UUID, so a repeated scan adds nothing and
/// a session restored, replaced or rewritten in any way is simply read again.
fn import_reader_host(store: &mut Store, request: &ImportRequest<'_>, host: Host) -> HostReport {
    match reader_sources_present(request.home, host) {
        Ok(true) => {}
        Ok(false) => {
            return HostReport::unavailable(
                host,
                HostStatus::MissingSource,
                format!("no {} session directory under the home", host.as_str()),
            );
        }
        Err(error) => {
            return HostReport::unavailable(
                host,
                HostStatus::ReaderFailed,
                format!("source root inaccessible: {}", error.kind()),
            );
        }
    }
    let python = match readers_cli::discover_python(request.python, request.cancel) {
        Ok(python) => python,
        Err(ReaderError::Cancelled) => {
            return HostReport::unavailable(
                host,
                HostStatus::Cancelled,
                "scan cancelled while the interpreter was probed",
            );
        }
        Err(error) => {
            return HostReport::unavailable(host, HostStatus::MissingRuntime, error.to_string());
        }
    };
    let producer = match request.producer.producer() {
        Ok(producer) => producer,
        Err(error) => {
            return HostReport::unavailable(host, HostStatus::PinMismatch, error.to_string());
        }
    };
    let (mut stdout, handle) =
        match readers_cli::spawn_reader(&python, &producer, host, request.home, request.cancel) {
            Ok(spawned) => spawned,
            Err(ReaderError::Cancelled) => {
                return HostReport::unavailable(
                    host,
                    HostStatus::Cancelled,
                    "scan cancelled before the reader started",
                );
            }
            Err(error) => {
                return HostReport::unavailable(host, HostStatus::ReaderFailed, error.to_string());
            }
        };
    let detail = format!(
        "pinned producer {} (memhub {})",
        producer.commit, producer.plugin_version
    );
    let lines = std::iter::from_fn(|| {
        let mut line = String::new();
        match std::io::BufRead::read_line(&mut stdout, &mut line) {
            Ok(0) => None,
            Ok(_) => Some(Ok(line)),
            Err(error) => Some(Err(error)),
        }
    });
    import_reader_lines(store, host, detail, lines, request.observed_at, move || {
        handle.finish()
    })
}

/// Import one host's stream as it arrives. Records are written in bounded
/// batches while a session is still being read, so memory holds at most one
/// batch. The session still open when the stream ends is committed only after
/// `finish` confirms a normal producer exit; if the producer failed, that
/// session's earlier batches stay but it is reported as ended early, without
/// a cursor, so it is never mistaken for a fully consumed source.
pub fn import_reader_lines<I, F>(
    store: &mut Store,
    host: Host,
    detail: String,
    lines: I,
    observed_at: i64,
    finish: F,
) -> HostReport
where
    I: IntoIterator<Item = std::io::Result<String>>,
    F: FnOnce() -> Result<readers_cli::ReaderOutcome, ReaderError>,
{
    struct Active {
        writer: SessionWriter,
        batch: Vec<crate::canonical::ParsedRecord>,
        cursor: SourceCursor,
    }
    let mut events = stream::StreamEvents::new(host, SessionSource::ReadersCli);
    let mut sessions: Vec<SessionResult> = Vec::new();
    let mut active: Option<Active> = None;
    let mut stream_failure: Option<String> = None;
    // Complete the session that a successor header (or a confirmed end) closed.
    fn complete(
        store: &mut Store,
        active: Active,
        observed_at: i64,
        sessions: &mut Vec<SessionResult>,
    ) {
        let Active {
            mut writer,
            batch,
            cursor,
        } = active;
        // A session that produced no storable record (a header-only or
        // dropped-only stream) exists only as a discovered identity: an empty
        // batch commits no row, and no cursor is recorded without a committed
        // batch.
        if let Err(skipped) = writer.write(store, &batch, observed_at) {
            sessions.push(*skipped);
            return;
        }
        sessions.push(writer.complete(store, Some(&cursor)));
    }
    let mut lines = lines.into_iter();
    for line in lines.by_ref() {
        let line = match line {
            Ok(line) => line,
            Err(_) => {
                stream_failure = Some("reader stream could not be read".to_owned());
                break;
            }
        };
        let text = line.trim_end_matches(['\n', '\r']);
        let event = match events.push(text) {
            Ok(Some(event)) => event,
            Ok(None) => continue,
            Err(error) => {
                stream_failure = Some(error.to_string());
                break;
            }
        };
        match event {
            stream::StreamEvent::Session(header) => {
                if let Some(previous) = active.take() {
                    complete(store, previous, observed_at, &mut sessions);
                }
                let cursor = SourceCursor {
                    source: SessionSource::ReadersCli,
                    cursor_key: format!("{}:{}", host.as_str(), header.path),
                    // Locator only. Incremental resume is not implemented.
                    position: 0,
                    updated_at: observed_at,
                };
                match SessionWriter::begin(
                    store,
                    host,
                    SessionSource::ReadersCli,
                    &header,
                    observed_at,
                ) {
                    Ok(writer) => {
                        active = Some(Active {
                            writer,
                            batch: Vec::new(),
                            cursor,
                        })
                    }
                    Err(skipped) => sessions.push(*skipped),
                }
            }
            stream::StreamEvent::Record(record) => {
                if let Some(session) = active.as_mut() {
                    session.batch.push(*record);
                    if session.batch.len() == MAX_BATCH_RECORDS {
                        let batch = std::mem::take(&mut session.batch);
                        if let Err(skipped) = session.writer.write(store, &batch, observed_at) {
                            sessions.push(*skipped);
                            active = None;
                        }
                    }
                }
            }
            stream::StreamEvent::Dropped => {
                if let Some(session) = active.as_mut() {
                    session.writer.note_dropped(1);
                }
            }
            stream::StreamEvent::MalformedHeader {
                native_session_id,
                line,
                reason,
            } => {
                // A boundary: the predecessor is complete; the successor is skipped.
                if let Some(previous) = active.take() {
                    complete(store, previous, observed_at, &mut sessions);
                }
                sessions.push(SessionResult {
                    conversation_id: native_session_id
                        .as_deref()
                        .map(|native| stream::expected_conversation_id(host, native)),
                    native_session_id,
                    source_surface: None,
                    path: None,
                    outcome: SessionOutcome::Skipped {
                        reason: format!("{reason} (stream line {line})"),
                    },
                });
            }
            stream::StreamEvent::MalformedRecord {
                native_session_id,
                line,
                reason,
            } => match active.take() {
                Some(session) => sessions.push(session.writer.abandon(format!(
                    "{reason} (stream line {line}); {} earlier batches stay committed",
                    session.writer.batches
                ))),
                None => sessions.push(SessionResult {
                    conversation_id: Some(stream::expected_conversation_id(
                        host,
                        &native_session_id,
                    )),
                    native_session_id: Some(native_session_id),
                    source_surface: None,
                    path: None,
                    outcome: SessionOutcome::Skipped {
                        reason: format!("{reason} (stream line {line})"),
                    },
                }),
            },
        }
    }
    if stream_failure.is_some() {
        // The producer may still be writing: drain its output to the end (or
        // to a read error) so it can exit, instead of waiting on a full pipe.
        for line in lines.by_ref() {
            if line.is_err() {
                break;
            }
        }
    }
    let outcome = finish();
    match (&stream_failure, &outcome) {
        (None, Ok(_)) => {
            if let Some(session) = active.take() {
                complete(store, session, observed_at, &mut sessions);
            }
        }
        _ => {
            if let Some(session) = active.take() {
                sessions.push(session.writer.abandon(format!(
                    "reader ended before completing this session; {} earlier batches stay committed",
                    session.writer.batches
                )));
            }
        }
    }
    let (status, detail, diagnostics) = match (stream_failure, outcome) {
        // A cancelled reader's stream ends wherever the kill landed: that end
        // is the cancellation, not a stream failure.
        (_, Err(ReaderError::Cancelled)) => (
            HostStatus::Cancelled,
            "reader cancelled before it finished; sessions it completed stay committed".into(),
            Vec::new(),
        ),
        (Some(failure), _) => (HostStatus::ReaderFailed, failure, Vec::new()),
        (None, Err(error)) => (HostStatus::ReaderFailed, error.to_string(), Vec::new()),
        (None, Ok(run)) => {
            let status = if run.complete && run.diagnostics.is_empty() && all_imported(&sessions) {
                HostStatus::Complete
            } else {
                HostStatus::Incomplete
            };
            (status, detail, run.diagnostics)
        }
    };
    HostReport {
        host,
        status,
        detail: Some(detail),
        diagnostics,
        sessions,
    }
}

/// One session's write sequence: the header registers the discovered
/// identity first; bounded record batches then commit one at a time, each
/// carrying the identity labels learned so far, which fill the discovered
/// identity only if that batch commits; and the cursor is recorded after the
/// last batch as a zero-position locator, only for a session with no rejected
/// or dropped record. It never supplies an incremental resume position. Titles are never
/// persisted: they derive from prompts.
pub struct SessionWriter {
    host: Host,
    native_session_id: String,
    context: crate::canonical::SourceContext,
    cwd: Option<String>,
    git_branch: Option<String>,
    result: SessionResult,
    new: usize,
    enriched: usize,
    rejected: Vec<String>,
    /// Batches committed so far; an ended-early session names this count.
    pub batches: usize,
}

impl SessionWriter {
    /// Register the header's identity. A conflicting discovered identity is an
    /// explicit skip before anything is written.
    pub fn begin(
        store: &mut Store,
        host: Host,
        source: SessionSource,
        header: &stream::SessionHeader,
        observed_at: i64,
    ) -> Result<Self, Box<SessionResult>> {
        let writer = Self {
            host,
            native_session_id: header.native_session_id.clone(),
            context: header.context(host, source),
            cwd: header.cwd.clone(),
            git_branch: header.git_branch.clone(),
            result: SessionResult {
                native_session_id: Some(header.native_session_id.clone()),
                conversation_id: Some(header.conversation_id.clone()),
                source_surface: None,
                path: Some(header.path.clone()),
                outcome: SessionOutcome::Imported {
                    records_new: 0,
                    records_enriched: 0,
                },
            },
            new: 0,
            enriched: 0,
            rejected: Vec::new(),
            batches: 0,
        };
        if header.native_session_id.trim().is_empty() || header.conversation_id.trim().is_empty() {
            return Err(Box::new(writer.abandon("session identity is blank".into())));
        }
        let mut identity = writer.discovery(observed_at);
        identity.surface = None;
        identity.started_at_ms = None;
        if let Err(error) = store.observe_discovered_session(&identity) {
            return Err(Box::new(writer.abandon(format!(
                "discovered identity conflicts with the index: {error}"
            ))));
        }
        Ok(writer)
    }

    /// The identity as known now: the header's, plus whichever labels records
    /// have revealed since.
    fn discovery(&self, observed_at: i64) -> DiscoveredSession {
        DiscoveredSession {
            host: self.host,
            native_session_id: self.native_session_id.clone(),
            conversation_id: self.context.conversation_id.clone(),
            surface: self.context.source_surface.clone(),
            started_at_ms: self
                .context
                .started_at
                .as_deref()
                .and_then(|value| xt_store::timestamp::parse(value).ok())
                .map(|(_, millis)| millis),
            last_observed_at: observed_at,
            // The identity is fully known from the header; host-level coverage is
            // reported separately.
            discovery_complete: true,
        }
    }

    /// Take in metadata that only records reveal (a Claude file's surface,
    /// cwd and branch). Each label fills once, from whichever batch first
    /// carries it. Nothing is persisted here: the labels travel with the next
    /// batch and fill the discovered identity only if that batch commits, so
    /// a batch the writer or the store rejects leaves no label behind.
    pub fn enrich(&mut self, header: &stream::SessionHeader) {
        if self.context.source_surface.is_none() {
            self.context.source_surface = header.source_surface.clone();
        }
        if self.context.started_at.is_none() {
            self.context.started_at = header.started_at.clone();
        }
        if self.cwd.is_none() {
            self.cwd = header.cwd.clone();
        }
        if self.git_branch.is_none() {
            self.git_branch = header.git_branch.clone();
        }
    }

    /// Resume behind a surface the index already holds for this session. It
    /// is persisted, so the report names it from the start; records behind
    /// the resume point carry it like a header's and must agree with it.
    pub fn resume_surface(&mut self, surface: String) {
        self.context.source_surface = Some(surface.clone());
        self.result.source_surface = Some(surface);
    }

    /// Whether every record-revealed label is known; until then each batch
    /// is inspected for the labels still missing.
    pub fn labels_complete(&self) -> bool {
        self.context.source_surface.is_some() && self.cwd.is_some() && self.git_branch.is_some()
    }

    /// Whether every record so far was stored; only then may progress advance.
    pub fn gapless(&self) -> bool {
        self.rejected.is_empty()
    }

    /// Records the parser could not identify; they count against coverage.
    pub fn note_dropped(&mut self, count: usize) {
        self.rejected
            .extend(std::iter::repeat_n("missing_uuid".to_owned(), count));
    }

    /// Commit one bounded batch together with the identity labels known so
    /// far; an empty batch commits nothing. A batch the writer or the store
    /// rejects leaves no row and no label behind.
    pub fn write(
        &mut self,
        store: &mut Store,
        records: &[crate::canonical::ParsedRecord],
        observed_at: i64,
    ) -> Result<(), Box<SessionResult>> {
        self.write_with_checkpoint(store, records, &[], observed_at, None)
    }

    /// `write`, with the structural hook summaries read alongside those records
    /// and resume progress; all three commit together or not at all.
    pub fn write_with_checkpoint(
        &mut self,
        store: &mut Store,
        records: &[crate::canonical::ParsedRecord],
        summaries: &[crate::canonical::StopHookSummary],
        observed_at: i64,
        checkpoint: Option<&NativeCheckpoint>,
    ) -> Result<(), Box<SessionResult>> {
        if records.is_empty() && summaries.is_empty() {
            return Ok(());
        }
        let discovery = self.discovery(observed_at);
        let batch = WriteBatch {
            context: &self.context,
            declared_host: Some(self.host),
            records,
            hook_summaries: summaries,
            title: None,
            cwd: self.cwd.as_deref(),
            git_branch: self.git_branch.as_deref(),
            namespace: None,
            // The persisted policy decides; an unconfigured store keeps metadata only.
            keep_content: true,
            observed_at,
            receipt: None,
            cursor: None,
            discovery: Some(&discovery),
            checkpoint,
        };
        match write_batch(store, &batch) {
            Ok(saved) => {
                self.new += saved.records_new;
                self.enriched += saved.records_enriched;
                // A rejection is a record this import could not store; it must
                // never read as complete coverage.
                self.rejected
                    .extend(saved.dropped_reasons.iter().map(
                        |(_, disposition)| match disposition {
                            RecordDisposition::RejectedOwnership => "rejected_ownership".to_owned(),
                            RecordDisposition::RejectedType => "rejected_type".to_owned(),
                            RecordDisposition::DroppedMissingUuid => "missing_uuid".to_owned(),
                            other => format!("{other:?}").to_lowercase(),
                        },
                    ));
                // The surface is reported once it is persisted.
                self.result.source_surface = self.context.source_surface.clone();
                self.batches += 1;
                Ok(())
            }
            Err(error) => Err(Box::new(self.abandon(format!(
                "session could not be written after {} committed batches: {error}",
                self.batches
            )))),
        }
    }

    /// Stop with an explicit reason; whatever earlier batches committed stays.
    pub fn abandon(&self, reason: String) -> SessionResult {
        SessionResult {
            outcome: SessionOutcome::Skipped { reason },
            ..self.result.clone()
        }
    }

    /// Record the cursor, then report. Empty complete scans reset old cursors.
    /// A new cursor is recorded only after at
    /// least one batch committed and only when no record was rejected or
    /// dropped, so it never claims the source was consumed past a gap; a
    /// session with a gap is reported partial and is read again next time.
    pub fn complete(mut self, store: &mut Store, cursor: Option<&SourceCursor>) -> SessionResult {
        if let Some(cursor) = cursor
            && self.rejected.is_empty()
        {
            if self.batches == 0 {
                if let Err(error) =
                    store.observe_discovered_session(&self.discovery(cursor.updated_at))
                {
                    return self.abandon(format!(
                        "completed header identity conflicts with the index: {error}"
                    ));
                }
                self.result.source_surface = self.context.source_surface.clone();
            }
            let saved = store.record_native_source_locator(cursor, self.batches > 0);
            if let Err(error) = saved {
                return self.abandon(format!(
                    "cursor could not be recorded after {} committed batches: {error}",
                    self.batches
                ));
            }
        }
        self.finish()
    }

    pub fn finish(self) -> SessionResult {
        let outcome = if self.rejected.is_empty() {
            SessionOutcome::Imported {
                records_new: self.new,
                records_enriched: self.enriched,
            }
        } else {
            SessionOutcome::Partial {
                records_new: self.new,
                records_enriched: self.enriched,
                records_dropped: self.rejected.len(),
                rejections: self.rejected,
            }
        };
        SessionResult {
            outcome,
            ..self.result
        }
    }
}
