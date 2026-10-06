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
pub mod claude_launch;
pub mod hook_names;
mod human_input;
pub mod origin;
pub mod readers_cli;
pub mod session_compactions;
pub mod session_creation;
pub mod session_source;
pub mod session_titles;
pub mod stream;
pub mod tool_sent;
pub mod watch;

use crate::writer::{MAX_BATCH_RECORDS, WriteBatch, write_batch_proven};
pub use readers_cli::{CancelToken, ProducerSource};
use readers_cli::{ReaderDiagnostic, ReaderError};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
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
    /// The Codex origin evidence this scan read, when it read the stream in
    /// origin-evidence mode; absent (and not serialized) otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginReport>,
}

impl HostReport {
    fn unavailable(host: Host, status: HostStatus, detail: impl Into<String>) -> Self {
        Self {
            host,
            status,
            detail: Some(detail.into()),
            diagnostics: Vec::new(),
            sessions: Vec::new(),
            origin: None,
        }
    }
}

/// What one origin-evidence scan made of the evidence: counts under fixed
/// names only, never a path, text, digest or title. Within one scan, every
/// validated claim ends in exactly one of `recorded`, `abstained`,
/// `disputed`, `store_refused`, `over_cap`, `unholdable` and
/// `unwritten`; a watcher's merged report adds its earlier passes'
/// `recorded` to the latest pass's counts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct OriginReport {
    pub human_inputs: xt_store::human_input::CorrectionReport,
    pub sessions: OriginSessions,
    pub claims: OriginClaims,
    /// The producer's advisory notices that a session's evidence was withheld,
    /// checked against the headers the stream carried.
    pub withheld_notices: WithheldNotices,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct OriginSessions {
    /// Headers carrying the version 1 marker: every record was examined.
    pub marked: usize,
    /// Headers without a marker: the session's evidence was withheld.
    pub withheld: usize,
    /// Headers whose marker is not the version 1 marker.
    pub invalid_marker: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct OriginClaims {
    /// Claims validated beside their record.
    pub claimed: usize,
    /// Proofs the Store kept, in the transaction that inserted their record.
    pub recorded: usize,
    pub abstained: OriginAbstained,
    /// Claims a later line of their session disputed: written without proof.
    pub disputed: usize,
    /// Proofs whose batch the Store refused; their records were written again
    /// without any proof.
    pub store_refused: usize,
    /// Claims beyond the per-session hold: written in order without proof.
    pub over_cap: usize,
    /// Claims whose record could change its batch or its session if moved (a
    /// label the header lacks or disagrees with, surface evidence, a cwd or
    /// branch the header leaves open, a value that cannot project): left in
    /// their ordinary batch without proof, where they write or fail exactly
    /// as without evidence.
    pub unholdable: usize,
    /// Claims whose session ended before its proofs could be written.
    pub unwritten: usize,
    /// Record lines whose evidence was refused, by fixed refusal code. They
    /// are not claims; each record still imported.
    pub refused: BTreeMap<&'static str, usize>,
}

/// Why the Store kept no proof although it wrote the claimed record.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct OriginAbstained {
    /// The record was already indexed (a replay) or not written by this input.
    pub not_inserted: usize,
    pub conflicted_record: usize,
    pub ineligible: usize,
    pub conflicting_proof: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct WithheldNotices {
    pub total: usize,
    /// Notices naming no header the stream carried.
    pub unmatched: usize,
    /// Notices naming a header that carried a marker.
    pub marked_conflict: usize,
    /// Unmarked headers no notice named.
    pub unreported: usize,
}

/// The producer's advisory code: a session's records are complete and
/// ordinary, only their origin evidence was withheld.
pub const ORIGIN_EVIDENCE_WITHHELD: &str = "origin_evidence_withheld";

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
    scan_native_continued(
        store,
        request,
        mode,
        observer,
        &mut session_creation::SpawnBacklog::default(),
        session_creation::spawn_limits(),
    )
}

/// `scan_native_observed` for a caller that keeps creation work between
/// scans: the Codex threads this scan indexed join `spawns`, one bounded pass
/// within `limits` runs over it, and what the pass could not reach stays
/// there for the caller to continue ([`session_creation::continue_codex_spawns`]).
pub fn scan_native_continued(
    store: &mut Store,
    request: &ImportRequest<'_>,
    mode: ScanMode,
    observer: &mut dyn FnMut(&Path),
    spawns: &mut session_creation::SpawnBacklog,
    limits: session_titles::TitleLimits,
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
            Host::Claude => {
                let report = import_claude(store, request, mode, observer);
                // A scan is a source change: sessions still checking are
                // looked at, and own reads that failed are made again.
                spawns.checks.wake();
                // A Claude session a Codex agent launched may be linked now.
                spawns.launches.add_children(imported(&report));
                // A Claude session may be a child a Claude agent launched, or
                // a caller whose launches now have their result.
                spawns.bash.add_sessions(imported(&report));
                report
            }
            Host::Codex => {
                let report = import_reader_host(store, request, Host::Codex);
                // Also when the reader could not run: a header read from the
                // index's own locators is asked for again, so a restored
                // root rollout is read without the reader.
                spawns.checks.wake();
                relate_codex_spawns(store, request, &report, spawns, limits);
                report
            }
            Host::Cursor => {
                let report = import_reader_host(store, request, Host::Cursor);
                spawns.checks.wake();
                report
            }
            Host::Other => HostReport::unavailable(
                Host::Other,
                HostStatus::MissingSource,
                "no native reader exists for an unknown host",
            ),
        })
        .collect();
    ImportReport { hosts }
}

/// After a Codex scan, record which threads were spawned by another: every
/// thread this scan indexed, whether or not it added records (an unchanged
/// transcript can still open with a new or different header, and a different
/// parent must conflict the relation), then the next part of the pass over
/// threads indexed before relations existed. That pass runs whatever the
/// reader's outcome, since it reads only what the index already holds.
///
/// Relations are display-only, so this never changes the scan's report. Work
/// the bounded pass does not reach, or that fails, stays in `spawns` and in
/// the pass's saved progress for a later pass. A sweep `spawns` holds is left
/// to those later passes; what this pass changes is published with the scan's
/// own event.
fn relate_codex_spawns(
    store: &mut Store,
    request: &ImportRequest<'_>,
    report: &HostReport,
    spawns: &mut session_creation::SpawnBacklog,
    limits: session_titles::TitleLimits,
) {
    if cancelled(request) || report.status == HostStatus::Cancelled {
        return;
    }
    spawns.add(imported(report));
    // Their histories may hold new launches, and each may be the child a
    // Codex agent's `codex exec --json` launch named.
    spawns.launches.add_threads(imported(report));
    spawns.launches.add_children(imported(report));
    let _ = session_creation::continue_scanned_spawns(
        store,
        request.home,
        spawns,
        limits,
        request.cancel,
        request.observed_at,
    );
}

/// The native identities of the sessions a host scan imported, in whole or
/// in part.
fn imported(report: &HostReport) -> impl Iterator<Item = &str> {
    report
        .sessions
        .iter()
        .filter(|session| {
            matches!(
                session.outcome,
                SessionOutcome::Imported { .. } | SessionOutcome::Partial { .. }
            )
        })
        .filter_map(|session| session.native_session_id.as_deref())
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
    let roots = [
        ".claude",
        ".codex",
        ".cursor",
        ".config/memhub-plugin",
        "Library/Application Support/Cursor/User/globalStorage",
    ]
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
        "Library/Application Support/Cursor/User/globalStorage",
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

#[cfg(test)]
mod preflight_tests {
    use super::*;
    use serde_json::json;
    use xt_store::{child_check::ChildCheck, session_list};

    #[test]
    fn a_changed_codex_header_reopens_before_the_next_record_is_published() {
        const NATIVE: &str = "019a0000-0000-7000-8000-00000000aa01";
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path();
        let db = home.join("index.sqlite");
        let path = home
            .join(".codex/sessions/2026/10/05")
            .join(format!("rollout-2026-10-05T15-17-30-{NATIVE}.jsonl"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let opening = |cwd: &str| {
            json!({"timestamp":"2026-10-05T15:17:30Z","type":"session_meta","ordinal":0,
                "payload":{"id":NATIVE,"session_id":NATIVE,"timestamp":"2026-10-05T15:17:30Z",
                    "cwd":cwd,"originator":"Codex Desktop","source":"cli",
                    "history_mode":"paginated"}})
        };
        let header = || {
            json!({"type":"session","host":"codex","native_session_id":NATIVE,
                "conversation_id":format!("codex-{NATIVE}"),"source_surface":"codex_cli",
                "started_at":"2026-10-05T15:17:30Z","cwd":"/w","git_branch":null,
                "title":null,"path":path.display().to_string(),"mtime":1.0})
            .to_string()
        };
        let record = |uuid: &str| {
            json!({"uuid":uuid,"type":"user","timestamp":"2026-10-05T15:17:31Z",
                "message":{"role":"user","content":[{"type":"text","text":"Synthetic"}]}})
            .to_string()
        };
        std::fs::write(&path, format!("{}\n", opening("/w"))).unwrap();
        let mut store = Store::open(&db).unwrap();
        let first = import_stream(
            &mut store,
            Host::Codex,
            "test".into(),
            [Ok(header()), Ok(record("first"))],
            1,
            || {
                Ok(readers_cli::ReaderOutcome {
                    diagnostics: vec![],
                    complete: true,
                })
            },
            Evidence::default(),
            None,
        );
        assert_eq!(first.status, HostStatus::Complete, "{first:?}");
        let checked = || {
            session_list::context(
                &rusqlite::Connection::open(&db).unwrap(),
                &[&format!("codex-{NATIVE}")],
            )
            .unwrap()
            .remove(0)
            .check
        };
        let probe = session_creation::record_codex_spawns(
            &mut store,
            home,
            &[NATIVE],
            session_creation::spawn_limits(),
            None,
            1,
        )
        .unwrap();
        let (_, attempt, observed_opening) = &probe.openings[0];
        store
            .settle_child_checks(&[(&format!("codex-{NATIVE}"), *attempt, &observed_opening.key)])
            .unwrap();
        assert_eq!(checked(), ChildCheck::Checked);

        std::fs::write(&path, format!("{}\n", opening("/x"))).unwrap();
        let mut seen = 0;
        let lines = [Ok(header()), Ok(record("second"))]
            .into_iter()
            .inspect(|_| {
                if seen == 1 {
                    assert_eq!(checked(), ChildCheck::Checking);
                    let count: i64 = rusqlite::Connection::open(&db)
                        .unwrap()
                        .query_row(
                            "SELECT count(*) FROM records WHERE session_id=?1",
                            [format!("codex-{NATIVE}")],
                            |row| row.get(0),
                        )
                        .unwrap();
                    assert_eq!(count, 1);
                }
                seen += 1;
            });
        let second = import_stream(
            &mut store,
            Host::Codex,
            "test".into(),
            lines,
            2,
            || {
                Ok(readers_cli::ReaderOutcome {
                    diagnostics: vec![],
                    complete: true,
                })
            },
            Evidence::default(),
            Some(home),
        );
        assert_eq!(second.status, HostStatus::Complete, "{second:?}");
        assert_eq!(checked(), ChildCheck::Checking);

        // The reader may next report another root path while the old path
        // still exists. An unchanged old header cannot keep completion for
        // records arriving from that newly reported source.
        let probe = session_creation::record_codex_spawns(
            &mut store,
            home,
            &[NATIVE],
            session_creation::spawn_limits(),
            None,
            2,
        )
        .unwrap();
        let (_, attempt, observed_opening) = &probe.openings[0];
        store
            .settle_child_checks(&[(&format!("codex-{NATIVE}"), *attempt, &observed_opening.key)])
            .unwrap();
        assert_eq!(checked(), ChildCheck::Checked);
        let relocated = path.with_file_name(format!("rollout-2026-10-05T15-18-30-{NATIVE}.jsonl"));
        std::fs::write(&relocated, format!("{}\n", opening("/x"))).unwrap();
        let mut moved_header: serde_json::Value = serde_json::from_str(&header()).unwrap();
        moved_header["path"] = json!(relocated.display().to_string());
        let mut seen = 0;
        let lines = [Ok(moved_header.to_string()), Ok(record("third"))]
            .into_iter()
            .inspect(|_| {
                if seen == 1 {
                    assert_eq!(checked(), ChildCheck::Checking);
                }
                seen += 1;
            });
        let third = import_stream(
            &mut store,
            Host::Codex,
            "test".into(),
            lines,
            3,
            || {
                Ok(readers_cli::ReaderOutcome {
                    diagnostics: vec![],
                    complete: true,
                })
            },
            Evidence::default(),
            Some(home),
        );
        assert_eq!(third.status, HostStatus::Complete, "{third:?}");
    }
}

fn import_claude(
    store: &mut Store,
    request: &ImportRequest<'_>,
    mode: ScanMode,
    observer: &mut dyn FnMut(&Path),
) -> HostReport {
    // A completed Claude row whose indexed own transcript disappeared must
    // stop drawing before Ready/Reconciled, even when enumeration cannot see
    // that missing file. This is a bounded metadata/open check, not a reread
    // of the source body or a reset of neighboring completed sessions.
    let mut after = None::<String>;
    loop {
        if cancelled(request) {
            return HostReport::unavailable(
                Host::Claude,
                HostStatus::Cancelled,
                "scan cancelled while checking indexed Claude sources",
            );
        }
        let page = match store.completed_claude_checks(after.as_deref(), 256) {
            Ok(page) => page,
            Err(error) => {
                return HostReport::unavailable(
                    Host::Claude,
                    HostStatus::ReaderFailed,
                    format!("indexed Claude checks could not be read: {error}"),
                );
            }
        };
        if page.is_empty() {
            break;
        }
        for (session, native) in &page {
            match claude_launch::bash::transcript_readable(store, request.home, native) {
                Ok(true) => {}
                Ok(false) => {
                    if let Err(error) = store.observe_own_check(session, None) {
                        return HostReport::unavailable(
                            Host::Claude,
                            HostStatus::ReaderFailed,
                            format!("indexed Claude check could not be withdrawn: {error}"),
                        );
                    }
                }
                Err(error) => {
                    return HostReport::unavailable(
                        Host::Claude,
                        HostStatus::ReaderFailed,
                        format!("indexed Claude source could not be checked: {error}"),
                    );
                }
            }
        }
        after = page.last().map(|(session, _)| session.clone());
    }
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
                    origin: None,
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
        origin: None,
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
    // Codex, and only Codex, is read with its origin evidence. Codex and
    // Cursor are both read with the automated-input evidence.
    let origin = host == Host::Codex;
    let tool_sent = matches!(host, Host::Codex | Host::Cursor);
    let spawned = if origin {
        readers_cli::spawn_codex_reader_with_origin_evidence(
            &python,
            &producer,
            request.home,
            request.cancel,
        )
    } else if tool_sent {
        readers_cli::spawn_cursor_reader_with_tool_sent_evidence(
            &python,
            &producer,
            request.home,
            request.cancel,
        )
    } else {
        readers_cli::spawn_reader(&python, &producer, host, request.home, request.cancel)
    };
    let (mut stdout, handle) = match spawned {
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
    import_stream(
        store,
        host,
        detail,
        lines,
        request.observed_at,
        move || handle.finish(),
        Evidence { origin, tool_sent },
        origin.then_some(request.home),
    )
}

/// Which opt-in reader evidence a stream carries.
#[derive(Clone, Copy, Default)]
struct Evidence {
    /// Codex origin evidence (with its image-wrapper lengths).
    origin: bool,
    /// Automated-input evidence ([`tool_sent`]).
    tool_sent: bool,
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
    import_stream(
        store,
        host,
        detail,
        lines,
        observed_at,
        finish,
        Evidence::default(),
        None,
    )
}

/// [`import_reader_lines`] for a stream read with the automated-input
/// evidence alone ([`readers_cli::spawn_cursor_reader_with_tool_sent_evidence`]).
/// Every record is written exactly as the ordinary import writes it, in the
/// same batches; a record whose own line carries a validated claim is written
/// with that claim's kind, which the Store keeps as a proof bound to the
/// stored record, new or already held (see [`tool_sent`]).
pub fn import_reader_lines_tool_sent<I, F>(
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
    import_stream(
        store,
        host,
        detail,
        lines,
        observed_at,
        finish,
        Evidence {
            origin: false,
            tool_sent: true,
        },
        None,
    )
}

/// [`import_reader_lines`] for a Codex stream read with its origin evidence
/// ([`readers_cli::spawn_codex_reader_with_origin_evidence`]). Every record
/// is written exactly as the ordinary import writes it, in the same bounded
/// batches; the records the evidence validly claims are held aside, at most
/// [`MAX_BATCH_RECORDS`] per session, and written with their proofs after
/// the session's last ordinary batch, in one Store transaction that keeps a
/// proof only with the record its own input inserts.
///
/// A claim a later line of its session disputes (the same record, item or
/// row, however malformed that line's claim) is written without its proof. A
/// session that ends early writes, without proofs, exactly the held records
/// its committed batches would have held, and drops the rest with its
/// discarded tail, so the committed records are those of the ordinary import.
/// A proof-bearing batch the Store refuses is written once more without any
/// proof. The producer's `origin_evidence_withheld` notices are advisory: they
/// are counted in [`HostReport::origin`], not listed as diagnostics, and do not
/// by themselves make the host incomplete. The same stream's automated-input
/// claims are read as [`import_reader_lines_tool_sent`] reads them.
pub fn import_reader_lines_origin<I, F>(
    store: &mut Store,
    detail: String,
    lines: I,
    observed_at: i64,
    finish: F,
) -> HostReport
where
    I: IntoIterator<Item = std::io::Result<String>>,
    F: FnOnce() -> Result<readers_cli::ReaderOutcome, ReaderError>,
{
    import_stream(
        store,
        Host::Codex,
        detail,
        lines,
        observed_at,
        finish,
        Evidence {
            origin: true,
            tool_sent: true,
        },
        None,
    )
}

/// A validly claimed record held until its session ends.
struct Held {
    record: crate::canonical::ParsedRecord,
    proof: xt_store::injected::InjectedContextProof,
}

/// One session's held claims, and where its committed batches stand. A
/// window is the run of records one ordinary batch covers, held ones
/// included, so batches end exactly where the ordinary import's do, and each
/// holds the ordinary import's records in their order, less the held ones.
#[derive(Default)]
struct Holding {
    /// Held claims by their position among the session's records.
    held: BTreeMap<usize, Held>,
    by_uuid: HashMap<String, usize>,
    /// Disputed claims whose window has committed: the ordinary import had
    /// written them, so they are written, without proof, before anything
    /// read after them.
    overdue: Vec<crate::canonical::ParsedRecord>,
    /// The session position of each record in the ordinary batch.
    batch_positions: Vec<usize>,
    /// Records of the session so far.
    position: usize,
    /// Records of the current window.
    window: usize,
    /// Records behind the last committed batch.
    watermark: usize,
}

impl Holding {
    /// Take one record in: a claim is held, while there is room and moving
    /// its record changes nothing else; anything else joins the ordinary
    /// batch.
    fn take(
        &mut self,
        batch: &mut Vec<crate::canonical::ParsedRecord>,
        writer: &SessionWriter,
        record: crate::canonical::ParsedRecord,
        claim: Option<Box<xt_store::injected::InjectedContextProof>>,
        claims: &mut OriginClaims,
    ) {
        let position = self.position;
        self.position += 1;
        self.window += 1;
        // A record whose place in a batch can matter stays in its ordinary
        // batch, without proof: it fills, disagrees or fails there exactly
        // as without evidence.
        let claim = match claim {
            Some(_) if !writer.holdable(&record) => {
                claims.unholdable += 1;
                None
            }
            claim => claim,
        };
        match claim {
            Some(proof) if self.held.len() + self.overdue.len() < MAX_BATCH_RECORDS => {
                if let Some(uuid) = record.canonical.uuid.clone() {
                    self.by_uuid.insert(uuid, position);
                }
                self.held.insert(
                    position,
                    Held {
                        record,
                        proof: *proof,
                    },
                );
            }
            claim => {
                if claim.is_some() {
                    claims.over_cap += 1;
                }
                batch.push(record);
                self.batch_positions.push(position);
            }
        }
    }

    /// Release each disputed held claim, without its proof, to where the
    /// ordinary import has it: back into the open window at its position, or,
    /// if its window has committed, ahead of the next batch.
    fn dispute(
        &mut self,
        batch: &mut Vec<crate::canonical::ParsedRecord>,
        uuids: Vec<String>,
        claims: &mut OriginClaims,
    ) {
        for uuid in uuids {
            let Some(position) = self.by_uuid.remove(&uuid) else {
                continue;
            };
            let Some(held) = self.held.remove(&position) else {
                continue;
            };
            claims.disputed += 1;
            if position < self.watermark {
                self.overdue.push(held.record);
            } else {
                let at = self.batch_positions.partition_point(|&p| p < position);
                batch.insert(at, held.record);
                self.batch_positions.insert(at, position);
            }
        }
    }

    fn window_full(&self) -> bool {
        self.window == MAX_BATCH_RECORDS
    }

    /// Write the overdue disputed records; they stay owed if that fails.
    fn write_overdue(
        &mut self,
        store: &mut Store,
        writer: &mut SessionWriter,
        observed_at: i64,
    ) -> Result<(), Box<SessionResult>> {
        writer.write(store, &self.overdue, observed_at)?;
        self.overdue.clear();
        Ok(())
    }

    /// The window's ordinary batch committed.
    fn committed(&mut self) {
        self.watermark = self.position;
        self.window = 0;
        self.batch_positions.clear();
    }

    /// The session ended early. The held records behind its committed
    /// batches are written without proof, as those batches wrote them in the
    /// ordinary import; the rest go with the discarded tail. The session is
    /// already reported as ended, so a failure here changes nothing further.
    fn settle_early(
        self,
        store: &mut Store,
        writer: &mut SessionWriter,
        observed_at: i64,
        claims: &mut OriginClaims,
    ) {
        let mut owed = self.overdue;
        for (position, held) in self.held {
            claims.unwritten += 1;
            if position < self.watermark {
                owed.push(held.record);
            }
        }
        let _ = writer.write(store, &owed, observed_at);
    }

    /// The session's last ordinary batch committed: write every held record
    /// with its proof, in one batch.
    fn settle(
        self,
        store: &mut Store,
        writer: &mut SessionWriter,
        observed_at: i64,
        claims: &mut OriginClaims,
    ) -> Result<(), Box<SessionResult>> {
        let (records, proofs): (Vec<_>, Vec<_>) = self
            .held
            .into_values()
            .map(|held| (held.record, Some(held.proof)))
            .unzip();
        match writer.write_proven(store, &records, &proofs, observed_at) {
            Ok(ProvenWrite::Kept(outcomes)) => {
                use xt_store::injected::{
                    InjectedContextAbstention as Why, InjectedContextDisposition as Kept,
                };
                let abstained = &mut claims.abstained;
                for outcome in &outcomes {
                    match outcome.disposition {
                        Kept::Recorded => claims.recorded += 1,
                        Kept::Abstained(Why::NotInserted) => abstained.not_inserted += 1,
                        Kept::Abstained(Why::ConflictedRecord) => abstained.conflicted_record += 1,
                        Kept::Abstained(Why::Ineligible(_)) => abstained.ineligible += 1,
                        Kept::Abstained(Why::ConflictingProof) => abstained.conflicting_proof += 1,
                    }
                }
                claims.unwritten += proofs.len().saturating_sub(outcomes.len());
                Ok(())
            }
            Ok(ProvenWrite::Refused) => {
                claims.store_refused += proofs.len();
                Ok(())
            }
            Err(skipped) => {
                claims.store_refused += proofs.len();
                Err(skipped)
            }
        }
    }
}

/// The origin-evidence scan's running account: its report, and the header
/// paths the advisory notices are checked against, which leave it only as
/// counts.
#[derive(Default)]
struct OriginTally {
    report: OriginReport,
    marked_paths: HashSet<String>,
    unmarked_paths: HashSet<String>,
}

impl OriginTally {
    fn session(&mut self, state: origin::SessionOrigin, path: &str) {
        let sessions = &mut self.report.sessions;
        match state {
            origin::SessionOrigin::Marked => {
                sessions.marked += 1;
                self.marked_paths.insert(path.to_owned());
            }
            origin::SessionOrigin::Withheld => {
                sessions.withheld += 1;
                self.unmarked_paths.insert(path.to_owned());
            }
            origin::SessionOrigin::Invalid => {
                sessions.invalid_marker += 1;
                self.marked_paths.insert(path.to_owned());
            }
        }
    }

    /// Take the producer's advisory notices out of its diagnostics, checking
    /// each against the headers; every other diagnostic stays.
    fn split(&mut self, diagnostics: Vec<ReaderDiagnostic>) -> Vec<ReaderDiagnostic> {
        let notices = &mut self.report.withheld_notices;
        let mut named = HashSet::new();
        let mut kept = Vec::new();
        for diagnostic in diagnostics {
            if diagnostic.code != ORIGIN_EVIDENCE_WITHHELD {
                kept.push(diagnostic);
                continue;
            }
            notices.total += 1;
            match diagnostic.path {
                Some(path) if self.unmarked_paths.contains(&path) => {
                    named.insert(path);
                }
                Some(path) if self.marked_paths.contains(&path) => notices.marked_conflict += 1,
                _ => notices.unmatched += 1,
            }
        }
        notices.unreported = self.unmarked_paths.len() - named.len();
        kept
    }
}

/// How a proof-bearing batch was written.
pub enum ProvenWrite {
    /// Written with its proofs; one outcome per proof, in input order.
    Kept(Vec<xt_store::injected::InjectedContextOutcome>),
    /// The Store refused the batch with its proofs, and it was written
    /// again without any.
    Refused,
}

#[allow(clippy::too_many_arguments)]
fn import_stream<I, F>(
    store: &mut Store,
    host: Host,
    detail: String,
    lines: I,
    observed_at: i64,
    finish: F,
    evidence: Evidence,
    preflight_home: Option<&Path>,
) -> HostReport
where
    I: IntoIterator<Item = std::io::Result<String>>,
    F: FnOnce() -> Result<readers_cli::ReaderOutcome, ReaderError>,
{
    struct Active {
        writer: SessionWriter,
        batch: Vec<crate::canonical::ParsedRecord>,
        cursor: SourceCursor,
        /// `Some` in origin-evidence mode.
        holding: Option<Holding>,
    }
    let origin = evidence.origin;
    let mut events = if origin {
        stream::StreamEvents::with_codex_origin_evidence()
    } else {
        stream::StreamEvents::new(host, SessionSource::ReadersCli)
    };
    if evidence.tool_sent {
        events = events.with_tool_sent_evidence();
    }
    let mut tally = origin.then(OriginTally::default);
    let mut sessions: Vec<SessionResult> = Vec::new();
    let mut active: Option<Active> = None;
    let mut stream_failure: Option<String> = None;
    // Complete the session that a successor header (or a confirmed end) closed.
    fn complete(
        store: &mut Store,
        active: Active,
        observed_at: i64,
        sessions: &mut Vec<SessionResult>,
        tally: &mut Option<OriginTally>,
    ) {
        let Active {
            mut writer,
            batch,
            cursor,
            mut holding,
        } = active;
        let claims = tally.as_mut().map(|tally| &mut tally.report.claims);
        // A session that produced no storable record (a header-only or
        // dropped-only stream) exists only as a discovered identity: an empty
        // batch commits no row, and no cursor is recorded without a committed
        // batch.
        let written = match holding.as_mut() {
            Some(holding) => holding.write_overdue(store, &mut writer, observed_at),
            None => Ok(()),
        }
        .and_then(|()| writer.write(store, &batch, observed_at));
        if let Err(skipped) = written {
            sessions.push(*skipped);
            if let (Some(holding), Some(claims)) = (holding, claims) {
                holding.settle_early(store, &mut writer, observed_at, claims);
            }
            if let Some(tally) = tally {
                tally.report.human_inputs.merge(&writer.human_report);
            }
            return;
        }
        if let (Some(mut holding), Some(claims)) = (holding, claims) {
            holding.committed();
            if let Err(skipped) = holding.settle(store, &mut writer, observed_at, claims) {
                sessions.push(*skipped);
                if let Some(tally) = tally {
                    tally.report.human_inputs.merge(&writer.human_report);
                }
                return;
            }
        }
        if let Some(tally) = tally {
            tally.report.human_inputs.merge(&writer.human_report);
        }
        sessions.push(writer.complete(store, Some(&cursor)));
    }
    // End a session early: report it, then settle what it held.
    fn end_early(
        store: &mut Store,
        active: Active,
        reason: impl FnOnce(&SessionWriter) -> SessionResult,
        observed_at: i64,
        sessions: &mut Vec<SessionResult>,
        tally: &mut Option<OriginTally>,
    ) {
        let Active {
            mut writer,
            holding,
            ..
        } = active;
        sessions.push(reason(&writer));
        if let (Some(holding), Some(tally)) = (holding, tally.as_mut()) {
            holding.settle_early(store, &mut writer, observed_at, &mut tally.report.claims);
        }
        if let Some(tally) = tally {
            tally.report.human_inputs.merge(&writer.human_report);
        }
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
        let (event, evidence) = match events.push_with_origin(text) {
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
                    complete(store, previous, observed_at, &mut sessions, &mut tally);
                }
                if let Some(home) = preflight_home {
                    let completed = match store.completed_child_check(&header.conversation_id) {
                        Ok(existing) => existing,
                        Err(error) => {
                            sessions.push(SessionResult {
                                native_session_id: Some(header.native_session_id.clone()),
                                conversation_id: Some(header.conversation_id.clone()),
                                source_surface: None,
                                path: None,
                                outcome: SessionOutcome::Skipped {
                                    reason: format!("display check could not be read: {error}"),
                                },
                            });
                            continue;
                        }
                    };
                    if completed {
                        // Compare the indexed root before any changed canonical
                        // record can commit. A failed comparison also withdraws
                        // an old completion until a later successful probe.
                        let reported_root = format!("codex:{}", header.path);
                        let indexed_root = matches!(
                            session_titles::codex_segment(
                                Path::new(&header.path),
                                &header.native_session_id,
                            ),
                            Some(session_titles::Segment::Root)
                        ) && session_titles::title_sources(
                            store,
                            Host::Codex,
                            &header.native_session_id,
                        )
                        .is_ok_and(|sources| {
                            sources.iter().any(|source| source.locator == reported_root)
                        });
                        let matched = indexed_root
                            && session_creation::record_codex_spawns(
                                store,
                                home,
                                &[&header.native_session_id],
                                session_creation::spawn_limits(),
                                None,
                                observed_at,
                            )
                            .is_ok_and(|result| {
                                result
                                    .probes
                                    .first()
                                    .is_some_and(|probe| probe.opening.is_some())
                            });
                        if !matched
                            && let Err(error) =
                                store.observe_own_check(&header.conversation_id, None)
                        {
                            sessions.push(SessionResult {
                                native_session_id: Some(header.native_session_id.clone()),
                                conversation_id: Some(header.conversation_id.clone()),
                                source_surface: None,
                                path: None,
                                outcome: SessionOutcome::Skipped {
                                    reason: format!("display check could not be recorded: {error}"),
                                },
                            });
                            continue;
                        }
                    }
                }
                if let (Some(tally), Some(origin::Origin::Session(state))) =
                    (tally.as_mut(), &evidence)
                {
                    tally.session(*state, &header.path);
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
                            holding: origin.then(Holding::default),
                        })
                    }
                    Err(skipped) => sessions.push(*skipped),
                }
            }
            stream::StreamEvent::Record(record) => {
                let mut claim = None;
                if let (Some(tally), Some(origin::Origin::Record(judged))) =
                    (tally.as_mut(), evidence)
                {
                    let claims = &mut tally.report.claims;
                    match judged {
                        origin::RecordOrigin::Claimed(proof) => {
                            claims.claimed += 1;
                            claim = Some(proof);
                        }
                        origin::RecordOrigin::Invalid(refused) => {
                            *claims.refused.entry(refused.code()).or_default() += 1;
                        }
                        origin::RecordOrigin::Declined | origin::RecordOrigin::Withheld => {}
                    }
                }
                let disputes = events.take_origin_disputes();
                let Some(session) = active.as_mut() else {
                    // No writer holds this session: its records are not
                    // written in either mode.
                    if let (Some(tally), Some(_)) = (tally.as_mut(), &claim) {
                        tally.report.claims.unwritten += 1;
                    }
                    continue;
                };
                let full = match session.holding.as_mut() {
                    Some(holding) => {
                        let claims = &mut tally.as_mut().expect("origin mode").report.claims;
                        holding.take(&mut session.batch, &session.writer, *record, claim, claims);
                        holding.dispute(&mut session.batch, disputes, claims);
                        holding.window_full()
                    }
                    None => {
                        session.batch.push(*record);
                        session.batch.len() == MAX_BATCH_RECORDS
                    }
                };
                if full {
                    let batch = std::mem::take(&mut session.batch);
                    let written = match session.holding.as_mut() {
                        Some(holding) => {
                            holding.write_overdue(store, &mut session.writer, observed_at)
                        }
                        None => Ok(()),
                    }
                    .and_then(|()| session.writer.write(store, &batch, observed_at));
                    match written {
                        Ok(()) => {
                            if let Some(holding) = session.holding.as_mut() {
                                holding.committed();
                            }
                        }
                        Err(skipped) => {
                            if let Some(session) = active.take() {
                                end_early(
                                    store,
                                    session,
                                    |_| *skipped,
                                    observed_at,
                                    &mut sessions,
                                    &mut tally,
                                );
                            }
                        }
                    }
                }
            }
            stream::StreamEvent::Dropped => {
                let disputes = events.take_origin_disputes();
                if let Some(session) = active.as_mut() {
                    session.writer.note_dropped(1);
                    if let (Some(holding), Some(tally)) = (session.holding.as_mut(), tally.as_mut())
                    {
                        holding.dispute(&mut session.batch, disputes, &mut tally.report.claims);
                    }
                }
            }
            stream::StreamEvent::MalformedHeader {
                native_session_id,
                line,
                reason,
            } => {
                // A boundary: the predecessor is complete; the successor is skipped.
                if let Some(previous) = active.take() {
                    complete(store, previous, observed_at, &mut sessions, &mut tally);
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
                Some(session) => end_early(
                    store,
                    session,
                    |writer| {
                        writer.abandon(format!(
                            "{reason} (stream line {line}); {} earlier batches stay committed",
                            writer.batches
                        ))
                    },
                    observed_at,
                    &mut sessions,
                    &mut tally,
                ),
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
                complete(store, session, observed_at, &mut sessions, &mut tally);
            }
        }
        _ => {
            if let Some(session) = active.take() {
                end_early(
                    store,
                    session,
                    |writer| {
                        writer.abandon(format!(
                            "reader ended before completing this session; {} earlier batches stay committed",
                            writer.batches
                        ))
                    },
                    observed_at,
                    &mut sessions,
                    &mut tally,
                );
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
            // In origin-evidence mode the advisory notices leave the
            // diagnostics; every other diagnostic keeps its meaning.
            let diagnostics = match tally.as_mut() {
                Some(tally) => tally.split(run.diagnostics),
                None => run.diagnostics,
            };
            let human_complete = tally
                .as_ref()
                .is_none_or(|t| t.report.human_inputs.failed == 0);
            let status = if run.complete
                && diagnostics.is_empty()
                && all_imported(&sessions)
                && human_complete
            {
                HostStatus::Complete
            } else {
                HostStatus::Incomplete
            };
            (status, detail, diagnostics)
        }
    };
    HostReport {
        host,
        status,
        detail: Some(detail),
        diagnostics,
        sessions,
        origin: tally.map(|tally| tally.report),
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
    human_rules: human_input::InputRules,
    human_report: xt_store::human_input::CorrectionReport,
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
            human_rules: human_input::InputRules::default(),
            human_report: Default::default(),
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
        self.write_with_checkpoint(store, records, &[], &[], observed_at, None)
    }

    /// `write`, with the structural hook summaries and exact PR witnesses read
    /// alongside those records and resume progress; all of them commit
    /// together or not at all.
    pub fn write_with_checkpoint(
        &mut self,
        store: &mut Store,
        records: &[crate::canonical::ParsedRecord],
        summaries: &[crate::canonical::StopHookSummary],
        witnesses: &[crate::writer::PrWitness],
        observed_at: i64,
        checkpoint: Option<&NativeCheckpoint>,
    ) -> Result<(), Box<SessionResult>> {
        self.commit(
            store,
            records,
            summaries,
            witnesses,
            &[],
            observed_at,
            checkpoint,
        )
        .map(|_| ())
        .map_err(|error| self.failed(error))
    }

    /// `write` with one optional injected context proof per record, aligned
    /// by input index. If the Store refuses the batch while it carries any
    /// proof, the same records are written once more without any, whatever
    /// the refusal; only if that fails too is the session abandoned.
    pub fn write_proven(
        &mut self,
        store: &mut Store,
        records: &[crate::canonical::ParsedRecord],
        proofs: &[Option<xt_store::injected::InjectedContextProof>],
        observed_at: i64,
    ) -> Result<ProvenWrite, Box<SessionResult>> {
        match self.commit(store, records, &[], &[], proofs, observed_at, None) {
            Ok(outcomes) => Ok(ProvenWrite::Kept(outcomes)),
            Err(_) if proofs.iter().any(Option::is_some) => {
                match self.commit(store, records, &[], &[], &[], observed_at, None) {
                    Ok(_) => Ok(ProvenWrite::Refused),
                    Err(error) => Err(self.failed(error)),
                }
            }
            Err(error) => Err(self.failed(error)),
        }
    }

    /// Whether moving this record out of its ordinary batch can change
    /// nothing else the session writes: its measurements project, it agrees
    /// with every identity label the header gives and fills none the header
    /// leaves open, it carries no surface evidence (which the header never
    /// supplies), and it names a cwd or branch only where the header already
    /// fills one first, so its own can only raise the conflict flag, wherever
    /// it is written. Checked without the Store, before a claimed record
    /// leaves its ordinary batch.
    fn holdable(&self, record: &crate::canonical::ParsedRecord) -> bool {
        let batch = |records| WriteBatch {
            context: &self.context,
            declared_host: Some(self.host),
            records,
            hook_summaries: &[],
            pr_witnesses: &[],
            title: None,
            cwd: self.cwd.as_deref(),
            git_branch: self.git_branch.as_deref(),
            namespace: None,
            keep_content: true,
            observed_at: 0,
            receipt: None,
            cursor: None,
            discovery: None,
            checkpoint: None,
        };
        let canonical = &record.canonical;
        canonical.surface_evidence.is_none()
            && (canonical.cwd.is_none() || self.cwd.is_some())
            && (canonical.git_branch.is_none() || self.git_branch.is_some())
            && match (
                crate::writer::resolve_session(&batch(std::slice::from_ref(record))),
                crate::writer::resolve_session(&batch(&[])),
            ) {
                (Ok(alone), Ok(header)) => {
                    alone == header && crate::writer::coverage(&alone.session_id, record).is_ok()
                }
                _ => false,
            }
    }

    fn failed(&self, error: xt_store::Error) -> Box<SessionResult> {
        Box::new(self.abandon(format!(
            "session could not be written after {} committed batches: {error}",
            self.batches
        )))
    }

    /// Commit one batch; on failure nothing is written and nothing here
    /// changes.
    #[allow(clippy::too_many_arguments)]
    fn commit(
        &mut self,
        store: &mut Store,
        records: &[crate::canonical::ParsedRecord],
        summaries: &[crate::canonical::StopHookSummary],
        witnesses: &[crate::writer::PrWitness],
        proofs: &[Option<xt_store::injected::InjectedContextProof>],
        observed_at: i64,
        checkpoint: Option<&NativeCheckpoint>,
    ) -> xt_store::Result<Vec<xt_store::injected::InjectedContextOutcome>> {
        if records.is_empty() && summaries.is_empty() && witnesses.is_empty() {
            return Ok(Vec::new());
        }
        let discovery = self.discovery(observed_at);
        let batch = WriteBatch {
            context: &self.context,
            declared_host: Some(self.host),
            records,
            hook_summaries: summaries,
            pr_witnesses: witnesses,
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
        // Worked out before the write, in input order as before, so the batch
        // keeps no person preview of an input whose adjustment will say only
        // part of it is a person's words; applied only once the rows commit.
        // The rules observe on a copy that replaces them only on success, so
        // a refused batch leaves the questions they remember unchanged.
        let mut rules = (self.host == Host::Codex).then(|| self.human_rules.clone());
        let adjustments = if let Some(rules) = rules.as_mut() {
            let session = self.context.conversation_id.as_deref().unwrap_or("");
            records
                .iter()
                .enumerate()
                .map(|(index, record)| {
                    rules
                        .observe(
                            session,
                            &record.canonical,
                            proofs.get(index).and_then(Option::as_ref),
                        )
                        .or_else(|| record.human_adjustment.clone())
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let adjusted = adjustments.iter().map(Option::is_some).collect::<Vec<_>>();
        match write_batch_proven(store, &batch, proofs, &adjusted) {
            Ok((saved, proven)) => {
                if let Some(rules) = rules {
                    self.human_rules = rules;
                    let adjustments = adjustments.into_iter().flatten().collect::<Vec<_>>();
                    match store.apply_human_input_adjustments(&adjustments) {
                        Ok(report) => self.human_report.merge(&report),
                        Err(_) => self.human_report.failed += adjustments.len(),
                    }
                }
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
                Ok(proven)
            }
            Err(error) => Err(error),
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
