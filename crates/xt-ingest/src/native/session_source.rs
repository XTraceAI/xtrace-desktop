//! Resolving and reading **one** Claude session's original local source, on
//! demand, for a detail view.
//!
//! The index keeps metrics and structural metadata; a transcript is not
//! persisted. When a person opens one session, this module finds that
//! session's own files under the home the index was built from, proves they
//! are still the generation the saved measurements were taken from, and hands
//! the caller the records **in memory**. Nothing here writes: no store row, no
//! cache, no copy of the bytes on disk, no log of what the session said. The
//! source is opened for reading only.
//!
//! Four rules shape every outcome:
//!
//! - **Never the wrong session.** Identity is established the way the importer
//!   established it — by the file the session owns. Anything short of a
//!   single, verified match is reported as unavailable rather than guessed at,
//!   and an alias is never followed to reach one.
//! - **Unavailable is an answer, and so is uncertainty.** Missing, moved,
//!   unreadable, replaced, ambiguous and too large each name themselves, and a
//!   part of the history that could not be listed is never reported as a
//!   session that is simply absent. The saved metrics are untouched either way.
//! - **Read the bytes that were verified.** Each file is snapshotted into
//!   bounded memory from the descriptor that was checked, and both that
//!   descriptor and the name are checked again — identity, length and change
//!   time — before a byte is parsed. Any mutation observed across the read,
//!   an append included, makes the source a replaced one rather than a
//!   transcript to show.
//! - **Bounded and cancellable.** Ceilings are cumulative over every file of
//!   the session and are taken before anything is allocated, and a cancel
//!   returns at once with no content at all.
//!
//! Codex and Cursor are served only when the caller supplies a verified
//! pinned producer and interpreter ([`DetailReader`]), through the producer's
//! exact-detail mode: one identified JSONL session, read whole in memory under
//! the producer's declared ceilings, with nothing staged on disk. The reader is
//! supervised by [`run_exact_detail`], and its output is taken only as a
//! complete validated success — exit `0`, nothing on stderr, exactly one
//! session whose header is the requested identity under this home — and then
//! parsed by the same stream parser indexing uses. Any other run is refused
//! with a closed reason and everything it wrote is discarded. A session whose
//! selected Cursor representation is a SQLite store is refused as such; its
//! transcript is never read in its place. Without a reader, those hosts are
//! refused before any interpreter or producer is looked for, so opening one
//! starts no process and writes no file.

use super::checkpoint::{self, FileIdentity, ResumeBasis};
use super::claude_fs::{self, ClaudeFile};
use super::readers_cli::{
    CancelToken, DetailBounds, DetailFailure, PinnedProducer, ReaderDiagnostic, run_exact_detail,
};
use super::stream::{StreamEvent, StreamEvents};
use crate::canonical::{Parsed, ParsedRecord};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use xt_store::{Host, SessionSource, Store, batch::NativeCheckpoint};

/// The most source bytes one opened session may read, over every file that
/// belongs to it. A transcript beyond this is reported as too large rather
/// than shown in part: a detail view that silently dropped the end of a
/// session would mislabel where the time went.
pub const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;

/// The most records one opened session may hold in memory at once.
pub const MAX_SESSION_RECORDS: usize = 200_000;

/// How much of a source is read per call while it is snapshotted. The read is
/// chunked so a cancel lands promptly inside a large file.
const READ_CHUNK: usize = 64 * 1024;

/// The ceilings one read observes. Defaults are the constants above; a caller
/// may lower them, and tests do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceLimits {
    pub max_bytes: u64,
    pub max_records: usize,
}

impl Default for SourceLimits {
    fn default() -> Self {
        Self {
            max_bytes: MAX_SESSION_BYTES,
            max_records: MAX_SESSION_RECORDS,
        }
    }
}

/// One locator the index recorded for a session, with the generation
/// checkpoint that locator was read under. Read from the store by
/// [`indexed_sources`]; metadata only, never content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedSource {
    /// The `source_cursors` key, e.g. `claude:/home/.claude/projects/p/s.jsonl`.
    pub locator: String,
    pub checkpoint: Option<NativeCheckpoint>,
}

impl IndexedSource {
    /// The local path inside a Claude locator key.
    pub(super) fn claude_path(&self) -> Option<&str> {
        self.locator.strip_prefix("claude:")
    }
}

/// What to open, and under what limits.
pub struct SessionSourceRequest<'a> {
    /// The home whose `.claude` tree is read: the same home the index was
    /// built from.
    pub home: &'a Path,
    /// The host the index recorded for this session. Only Claude is served.
    pub host: Host,
    /// The native session identity to display, exactly. An identifier that
    /// could name a path instead of a session is refused.
    pub native_session_id: &'a str,
    /// The locators the index recorded for this session, from
    /// [`indexed_sources`]. Empty means the index recorded none, which is
    /// honest history, not an error.
    pub indexed: &'a [IndexedSource],
    /// Cancels this read: the file being read is abandoned and the outcome is
    /// `Cancelled`, with no content.
    pub cancel: Option<&'a CancelToken>,
    pub limits: SourceLimits,
}

/// Which file of a session was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceRole {
    /// The session's own transcript.
    Primary,
    /// A subagent transcript the host files under the session.
    Sidechain,
}

/// One file this read consumed. Paths stay on the local side of the contract:
/// they are index metadata, not view data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadSource {
    pub path: PathBuf,
    pub role: SourceRole,
    pub bytes: u64,
}

/// How the read relates to the generation the index measured, over every file
/// of the session together: one file that cannot be tied to a measurement
/// leaves the whole session untied, and otherwise one file that has grown
/// makes the session a grown one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationBasis {
    /// The bytes the index read are still the first bytes of every file.
    /// `appended` means a file has grown since; what the index measured is
    /// unchanged inside it.
    Indexed { appended: bool },
    /// The index recorded no checkpoint for at least one file, so the read
    /// cannot be tied to the measurement. The identity is still the session's.
    Unrecorded,
}

impl GenerationBasis {
    /// Fold one more file's basis into the session's.
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unrecorded, _) | (_, Self::Unrecorded) => Self::Unrecorded,
            (Self::Indexed { appended: a }, Self::Indexed { appended: b }) => {
                Self::Indexed { appended: a || b }
            }
        }
    }
}

/// Something the read could not cover, inside an otherwise usable session.
/// A gap is always named; it is never silently dropped from the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceGap {
    /// A file of this session could not be opened or read.
    Unreadable { path: PathBuf },
    /// A file of this session no longer holds the generation the index read,
    /// or changed while it was being read.
    Replaced { path: PathBuf, basis: ResumeBasis },
    /// A file stopped at a line that does not meet the canonical contract, as
    /// it would have stopped the importer.
    Stopped {
        path: PathBuf,
        line: u64,
        reason: &'static str,
    },
    /// A subagent transcript the index read is no longer where it read it. Its
    /// records are inside the saved measurements, so a session without it is
    /// not the session those numbers describe. It is reported, never hunted
    /// for elsewhere: this read follows nothing that moved.
    Missing { path: PathBuf },
    /// Part of the local history could not be listed, so this session's set of
    /// files may be incomplete. What was read is still this session's.
    DiscoveryIncomplete,
}

/// Which ceiling a read reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceCeiling {
    Bytes,
    Records,
}

/// One session's transient content, with everything the caller needs to say
/// how complete it is. Dropping this value is the whole of its lifecycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedSession {
    pub native_session_id: String,
    pub host: Host,
    pub generation: GenerationBasis,
    pub sources: Vec<ReadSource>,
    /// The session's records in read order: the primary transcript, then each
    /// subagent file in path order, as the importer attributed them.
    pub records: Vec<ParsedRecord>,
    /// Lines that carry no record: inert lines, structural summaries and PR
    /// witnesses. Counted so a caller can say the text is not the whole file.
    pub other_lines: u64,
    /// Records the canonical parser accepted but could not identify.
    pub dropped_records: u64,
    pub gaps: Vec<SourceGap>,
}

/// Why a session's original source may not be shown. Every variant leaves the
/// saved measurements standing: this says the text is unavailable, never that
/// the session did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceUnavailable {
    /// Nothing under the home names this session any more.
    Missing,
    /// The session's source is no longer where the index read it. The place it
    /// is now is known but is not read: the saved measurements were taken from
    /// the recorded locator, and this read does not silently follow a move.
    Moved { from: PathBuf, found: PathBuf },
    /// The session's own transcript is no longer the generation the index
    /// measured — replaced, truncated or rewritten — or it changed while it
    /// was being read.
    Replaced { basis: ResumeBasis },
    /// The session's own transcript, or the history that would hold it, could
    /// not be read. `detail` names a failure kind, never a path.
    Unreadable { detail: String },
    /// More than one source names this session. A guess between them could
    /// show the wrong one, so none is shown.
    Ambiguous { candidates: usize },
    /// The session is larger than this view may hold. `reached` and `ceiling`
    /// are counted in whatever `limit` names, so a record ceiling is never
    /// reported as a byte figure.
    TooLarge {
        limit: SourceCeiling,
        reached: u64,
        ceiling: u64,
    },
    /// The caller cancelled; no content is returned.
    Cancelled,
    /// The identifier could not name a session (empty, a path, a control
    /// character, or the reader's own `latest` selector).
    InvalidIdentifier,
    /// This host's sources are only reachable through the pinned reader, and
    /// no verified reader was supplied, so the session is refused before any
    /// interpreter or producer is looked for: no process starts and no file is
    /// written.
    PrerequisiteUnavailable,
    /// The pinned reader's exact-detail read did not produce this session.
    /// Every variant is closed: a declared ceiling, the deadline, a SQLite
    /// store this mode is not defined over, a source that changed, a refusal,
    /// or a broken protocol. Nothing the reader wrote is kept.
    Reader(DetailFailure),
    /// No native reader exists for this host at all.
    UnsupportedHost,
}

/// The whole result of opening one session's source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionSourceOutcome {
    Loaded(Box<LoadedSession>),
    Unavailable(SourceUnavailable),
}

impl SessionSourceOutcome {
    fn unavailable(reason: SourceUnavailable) -> Self {
        Self::Unavailable(reason)
    }
}

/// The locators the index recorded for one session.
///
/// Claude sessions own their files by name, so the lookup is exact: the
/// session's transcript and the subagent files filed under it. Other hosts are
/// not served by this module, and their result is empty.
pub fn indexed_sources(
    store: &Store,
    host: Host,
    native_session_id: &str,
) -> xt_store::Result<Vec<IndexedSource>> {
    if host != Host::Claude || identifier_error(native_session_id).is_some() {
        return Ok(Vec::new());
    }
    let escaped = xt_store::batch::escape_like(native_session_id);
    let patterns = [
        format!("claude:%/{escaped}.jsonl"),
        format!("claude:%/{escaped}/subagents/%"),
    ];
    let patterns: Vec<&str> = patterns.iter().map(String::as_str).collect();
    store
        .source_cursors_like(SessionSource::Transcript, &patterns)?
        .into_iter()
        .map(|cursor| {
            Ok(IndexedSource {
                checkpoint: store
                    .native_checkpoint(SessionSource::Transcript, &cursor.cursor_key)?,
                locator: cursor.cursor_key,
            })
        })
        .collect()
}

/// Whether an identifier could name one session here. A caller that must do
/// work of its own before a read — resolve an interpreter, say — asks this
/// first, so an identifier the read would refuse costs nothing.
pub fn names_one_session(id: &str) -> bool {
    identifier_error(id).is_none()
}

/// An identifier that cannot name one session, and why. A session identifier
/// becomes a file name, so a separator, a traversal step, a control character
/// or the reader's `latest` keyword would select something other than the
/// session that was asked for.
pub(super) fn identifier_error(id: &str) -> Option<SourceUnavailable> {
    let refused = id.is_empty()
        || id.len() > 255
        || id == "latest"
        || id == "."
        || id == ".."
        || id.starts_with('.')
        || id.contains(['/', '\\'])
        || id.chars().any(char::is_control)
        || id.trim() != id;
    refused.then_some(SourceUnavailable::InvalidIdentifier)
}

/// The verified pinned producer, and the interpreter to run it with, that
/// serve Codex and Cursor sessions. The caller verifies both before a read:
/// this module starts no probe and verifies no pin of its own.
#[derive(Clone, Copy, Debug)]
pub struct DetailReader<'a> {
    pub python: &'a OsStr,
    pub producer: &'a PinnedProducer,
    pub bounds: DetailBounds,
}

/// Open one Claude session's original local source and read it into memory.
///
/// Returns either the session's own records or the one reason they may not be
/// shown. Nothing is written, cached or logged, and the source bytes are
/// unchanged by the call.
pub fn load_session_source(request: &SessionSourceRequest<'_>) -> SessionSourceOutcome {
    load_session_source_with(request, None)
}

/// [`load_session_source`], with the pinned reader that serves Codex and
/// Cursor sessions when one is supplied.
pub fn load_session_source_with(
    request: &SessionSourceRequest<'_>,
    reader: Option<&DetailReader<'_>>,
) -> SessionSourceOutcome {
    // The host is settled before anything else: a host this contract cannot
    // serve must not cause an interpreter probe, a producer verification or a
    // temporary file, so it is refused here rather than deeper down.
    match request.host {
        Host::Claude => {}
        Host::Codex | Host::Cursor => {
            // An identity that could name a path, or the `latest` ranking, is
            // refused before any process starts: the reader would otherwise
            // read a file this session does not own.
            if let Some(refused) = identifier_error(request.native_session_id) {
                return SessionSourceOutcome::unavailable(refused);
            }
            let Some(reader) = reader else {
                return SessionSourceOutcome::unavailable(
                    SourceUnavailable::PrerequisiteUnavailable,
                );
            };
            if cancelled(request.cancel) {
                return SessionSourceOutcome::unavailable(SourceUnavailable::Cancelled);
            }
            return load_via_reader(request, reader);
        }
        Host::Other => {
            return SessionSourceOutcome::unavailable(SourceUnavailable::UnsupportedHost);
        }
    }
    if let Some(refused) = identifier_error(request.native_session_id) {
        return SessionSourceOutcome::unavailable(refused);
    }
    if cancelled(request.cancel) {
        return SessionSourceOutcome::unavailable(SourceUnavailable::Cancelled);
    }
    load_claude(request)
}

/// Read one Codex or Cursor session through the pinned reader's exact-detail
/// mode, and keep its records only if the whole stream is this session.
fn load_via_reader(
    request: &SessionSourceRequest<'_>,
    reader: &DetailReader<'_>,
) -> SessionSourceOutcome {
    // The reader runs inside the home, so the home is settled into one
    // absolute spelling first; a relative one would be resolved again beneath
    // itself once the reader had entered it.
    let home = match request.home.canonicalize() {
        Ok(home) if home.is_dir() => home,
        Ok(_) => return SessionSourceOutcome::unavailable(SourceUnavailable::Missing),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return SessionSourceOutcome::unavailable(SourceUnavailable::Missing);
        }
        Err(error) => {
            return SessionSourceOutcome::unavailable(SourceUnavailable::Unreadable {
                detail: format!("history home could not be resolved: {}", error.kind()),
            });
        }
    };
    // Where this host's sources may be, anchored before the reader runs, as
    // the reader anchors them: each configured root as spelled under the home
    // and, when it is a symlink, where it led. A root retargeted during the
    // read cannot vouch for a path under its new target.
    let roots = host_roots(request.host, &home);
    let stream = match run_exact_detail(
        reader.python,
        reader.producer,
        request.host,
        &home,
        request.native_session_id,
        reader.bounds,
        request.cancel,
    ) {
        Ok(stream) => stream,
        Err(DetailFailure::Cancelled) => {
            return SessionSourceOutcome::unavailable(SourceUnavailable::Cancelled);
        }
        Err(failure) => {
            return SessionSourceOutcome::unavailable(SourceUnavailable::Reader(failure));
        }
    };
    match session_from_stream(request, &roots, &stream) {
        Ok(loaded) => SessionSourceOutcome::Loaded(Box::new(loaded)),
        Err(reason) => SessionSourceOutcome::unavailable(reason),
    }
}

/// The configured roots the pinned reader discovers this host's sources under
/// (`~/.codex/sessions`; `~/.cursor/chats` and `~/.cursor/projects`), each as
/// spelled under the home and, where it resolves, where it leads now. The
/// reader supports a root that is a symlink to elsewhere and reports sources
/// by their resolved path, so the home itself is not the boundary: these are.
pub(super) fn host_roots(host: Host, home: &Path) -> Vec<PathBuf> {
    let relative: &[&str] = match host {
        Host::Codex => &[".codex/sessions"],
        Host::Cursor => &[".cursor/chats", ".cursor/projects"],
        Host::Claude | Host::Other => &[],
    };
    let mut roots = Vec::new();
    for root in relative.iter().map(|relative| home.join(relative)) {
        if let Ok(anchored) = root.canonicalize()
            && anchored != root
        {
            roots.push(anchored);
        }
        roots.push(root);
    }
    roots
}

/// Whether a reported source sits under one of the anchored roots: an
/// absolute path with no `.` or `..` step, compared component by component.
pub(super) fn under_roots(path: &Path, roots: &[PathBuf]) -> bool {
    use std::path::Component;
    path.is_absolute()
        && path
            .components()
            .all(|part| !matches!(part, Component::CurDir | Component::ParentDir))
        && roots.iter().any(|root| path.starts_with(root))
}

/// A stream that is not exactly one session, whole, is a broken protocol.
fn broken(reason: &'static str) -> SourceUnavailable {
    SourceUnavailable::Reader(DetailFailure::Protocol(reason))
}

/// Parse a complete exact-detail stream with the parser indexing uses, and
/// accept it only as exactly the requested session: one header naming this
/// host and identity, from a source under one of this host's anchored roots,
/// followed by records that all belong to it. Any other shape refuses the
/// whole stream; no part of it is returned, and a cancel that lands at any
/// point — after the last line included — returns nothing.
fn session_from_stream(
    request: &SessionSourceRequest<'_>,
    roots: &[PathBuf],
    stream: &[u8],
) -> Result<LoadedSession, SourceUnavailable> {
    let native = request.native_session_id;
    let text = std::str::from_utf8(stream).map_err(|_| broken("reader output is not UTF-8"))?;
    // The producer ends every line it writes; output that stops inside one
    // was cut short, whatever the exit status said.
    if !text.ends_with('\n') {
        return Err(broken("reader output ends inside a line"));
    }
    let mut events = StreamEvents::new(request.host, xt_store::SessionSource::ReadersCli);
    let mut budget = Budget::new(request.limits);
    let mut header = None;
    let mut records = Vec::new();
    let mut dropped_records = 0_u64;
    for line in text.split_terminator('\n') {
        if cancelled(request.cancel) {
            return Err(SourceUnavailable::Cancelled);
        }
        let event = events
            .push(line.trim_end_matches('\r'))
            .map_err(|_| broken("a record precedes the session header"))?;
        match event {
            None => {}
            Some(StreamEvent::Session(found)) => {
                if header.is_some() {
                    return Err(broken("reader wrote more than one session"));
                }
                // The parser has already held the header to this host and to a
                // conversation ID derived from its own native identity.
                if found.native_session_id != native {
                    return Err(broken("session header names another session"));
                }
                if !under_roots(Path::new(&found.path), roots) {
                    return Err(broken("session source is outside this host's roots"));
                }
                header = Some(*found);
            }
            Some(StreamEvent::Record(record)) => {
                if disagreeing_identity(&record)
                    || [
                        record.native.session_id.as_deref(),
                        record.canonical.native_session_id.as_deref(),
                    ]
                    .into_iter()
                    .flatten()
                    .any(|id| id != native)
                {
                    return Err(broken("a record names another session"));
                }
                budget
                    .take_record()
                    .map_err(|limit| budget.too_large(limit))?;
                records.push(*record);
            }
            Some(StreamEvent::Dropped) => dropped_records += 1,
            Some(StreamEvent::MalformedHeader { .. } | StreamEvent::MalformedRecord { .. }) => {
                return Err(broken(
                    "reader output does not match the shared stream contract",
                ));
            }
        }
        parsed_for_test(request.cancel);
    }
    // The last line may have been parsed after a cancel landed; the caller
    // was told nothing will come, so nothing does.
    if cancelled(request.cancel) {
        return Err(SourceUnavailable::Cancelled);
    }
    let header = header.ok_or_else(|| broken("reader wrote no session"))?;
    Ok(LoadedSession {
        native_session_id: native.to_owned(),
        host: request.host,
        // The reader proves its own source did not change while it read; the
        // index recorded no checkpoint this read could be tied to.
        generation: GenerationBasis::Unrecorded,
        sources: vec![ReadSource {
            path: PathBuf::from(header.path),
            role: SourceRole::Primary,
            // The verified stream's length: the reader reads the session's
            // files, and this process reads only what it wrote.
            bytes: stream.len() as u64,
        }],
        records,
        other_lines: 0,
        dropped_records,
        gaps: Vec::new(),
    })
}

fn cancelled(cancel: Option<&CancelToken>) -> bool {
    cancel.is_some_and(CancelToken::is_cancelled)
}

/// The running totals one read may not exceed, counted over every file of the
/// session together.
struct Budget {
    bytes: u64,
    records: usize,
    limits: SourceLimits,
}

impl Budget {
    fn new(limits: SourceLimits) -> Self {
        Self {
            bytes: 0,
            records: 0,
            limits,
        }
    }

    /// Charge a file's length before anything is allocated for it.
    fn take_bytes(&mut self, bytes: u64) -> Result<(), SourceCeiling> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > self.limits.max_bytes {
            return Err(SourceCeiling::Bytes);
        }
        Ok(())
    }

    fn take_record(&mut self) -> Result<(), SourceCeiling> {
        self.records += 1;
        if self.records > self.limits.max_records {
            return Err(SourceCeiling::Records);
        }
        Ok(())
    }

    /// The ceiling that stopped this read, counted in its own unit.
    fn too_large(&self, limit: SourceCeiling) -> SourceUnavailable {
        let (reached, ceiling) = match limit {
            SourceCeiling::Bytes => (self.bytes, self.limits.max_bytes),
            SourceCeiling::Records => (
                self.records as u64,
                u64::try_from(self.limits.max_records).unwrap_or(u64::MAX),
            ),
        };
        SourceUnavailable::TooLarge {
            limit,
            reached,
            ceiling,
        }
    }
}

/// Why one file of a session could not be read.
enum ReadFailure {
    /// The file is not the generation the index measured, or it changed while
    /// it was being read.
    Replaced(ResumeBasis),
    Unreadable(String),
    /// A ceiling was reached at or before this file.
    TooLarge(SourceCeiling),
    Cancelled,
}

/// What one file of a session yielded.
struct FileOutcome {
    records: Vec<ParsedRecord>,
    other_lines: u64,
    dropped_records: u64,
    bytes: u64,
    generation: GenerationBasis,
    /// The line at which the file stopped meeting the canonical contract, as
    /// it would have stopped the importer. Everything before it was read.
    stopped: Option<(u64, &'static str)>,
}

/// Where a Claude transcript sits below `~/.claude/projects`: its project
/// directory and everything under it. Two spellings of one home (an alias, a
/// temporary directory) name the same transcript by this suffix, while a
/// transcript that really moved changes it.
fn history_key(path: &Path) -> Option<&str> {
    const PROJECTS: &str = "/.claude/projects/";
    let text = path.to_str()?;
    text.rfind(PROJECTS).map(|at| &text[at + PROJECTS.len()..])
}

/// Which subagent transcript a path is, inside its own session: everything
/// below `<session>/subagents/`. A project directory renamed above it does not
/// change this, so a session whose files moved together is not mistaken for a
/// session that lost one.
fn sidechain_key<'a>(path: &'a Path, session_id: &str) -> Option<&'a str> {
    let text = path.to_str()?;
    let marker = format!("/{session_id}/subagents/");
    text.rfind(&marker).map(|at| &text[at + marker.len()..])
}

/// Whether two spellings name the same transcript. A path that does not sit
/// under a projects root at all is compared whole rather than guessed about.
fn same_transcript(left: &Path, right: &Path) -> bool {
    match (history_key(left), history_key(right)) {
        (Some(left), Some(right)) => left == right,
        _ => left == right,
    }
}

/// What a look through the home found, and what it could not see.
struct Discovery {
    primary: Vec<ClaudeFile>,
    sidechains: Vec<ClaudeFile>,
    /// Somewhere that could hold a transcript named after this session could
    /// not be looked at. That is not only an absent session that cannot be
    /// claimed: it is an *ambiguity* that cannot be ruled out, because the
    /// place unlooked-at could hold another file of the same name. A single
    /// candidate beside one of these is not known to be the only one.
    primary_uncertain: bool,
    /// The session's own subagent tree could not be listed completely. Which
    /// session the files belong to is not in doubt — only how many there are —
    /// so this is a gap in what loads, not a reason to refuse it.
    sidechain_uncertain: bool,
}

/// Every place under this home that names the session: its own transcript in
/// each project directory, and the subagent files filed under it.
///
/// Nothing is opened here, and no alias is followed — not a project
/// directory's, not a transcript's, and not the session directory's, whose
/// target could file a foreign tree's transcripts under this session. What
/// could not be listed is remembered rather than passed over.
fn discover_claude(projects: &Path, session_id: &str) -> std::io::Result<Discovery> {
    let mut found = Discovery {
        primary: Vec::new(),
        sidechains: Vec::new(),
        primary_uncertain: false,
        sidechain_uncertain: false,
    };
    for entry in fs::read_dir(projects)? {
        let project = match entry {
            Ok(entry) => entry.path(),
            // A directory entry that cannot even be named could be a project
            // holding this session; the others are still searched.
            Err(_) => {
                found.primary_uncertain = true;
                continue;
            }
        };
        match fs::symlink_metadata(&project) {
            // An alias is never followed, here or in the importer, and a
            // non-directory cannot hold a project's transcripts.
            Ok(meta) if meta.file_type().is_symlink() => {
                found.primary_uncertain = true;
                continue;
            }
            Ok(meta) if !meta.is_dir() => continue,
            Ok(_) => {}
            Err(_) => {
                found.primary_uncertain = true;
                continue;
            }
        }
        match claude_fs::observe(
            project.join(format!("{session_id}.jsonl")),
            session_id.to_owned(),
            false,
        ) {
            Ok(file) => found.primary.push(file),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            // Present but not observable as a regular file: an alias, a
            // device, or an entry this process cannot stat. Something there
            // bears this session's name and could not be ruled in or out.
            Err(_) => found.primary_uncertain = true,
        }
        collect_sidechains(&project.join(session_id), session_id, &mut found);
    }
    found.sidechains.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// The subagent transcripts filed under one session directory.
///
/// The session directory itself must be a real directory: an alias there would
/// let another tree's transcripts be read as this session's subagents, which
/// is the same mistake as following an alias to a transcript.
fn collect_sidechains(session_directory: &Path, session_id: &str, found: &mut Discovery) {
    for step in [session_directory, &session_directory.join("subagents")] {
        match fs::symlink_metadata(step) {
            Ok(meta) if meta.file_type().is_symlink() => {
                found.sidechain_uncertain = true;
                return;
            }
            Ok(meta) if meta.is_dir() => {}
            // No subagent tree for this session is ordinary.
            Ok(_) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => {
                found.sidechain_uncertain = true;
                return;
            }
        }
    }
    let mut diagnostics: Vec<ReaderDiagnostic> = Vec::new();
    claude_fs::collect_jsonl(
        &session_directory.join("subagents"),
        session_id,
        &mut found.sidechains,
        &mut diagnostics,
    );
    // The enumeration reports what it could not list or would not follow; a
    // session whose subagent tree is partly unreadable is not a complete one.
    found.sidechain_uncertain |= !diagnostics.is_empty();
}

fn load_claude(request: &SessionSourceRequest<'_>) -> SessionSourceOutcome {
    let projects = request.home.join(".claude").join("projects");
    let mut found = match discover_claude(&projects, request.native_session_id) {
        Ok(found) => found,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return SessionSourceOutcome::unavailable(SourceUnavailable::Missing);
        }
        Err(error) => {
            return SessionSourceOutcome::unavailable(SourceUnavailable::Unreadable {
                detail: format!("Claude history could not be listed: {}", error.kind()),
            });
        }
    };
    if found.primary.len() > 1 {
        return SessionSourceOutcome::unavailable(SourceUnavailable::Ambiguous {
            candidates: found.primary.len(),
        });
    }
    // Somewhere that could hold a transcript of this name could not be looked
    // at. Whether the session is absent and whether the one candidate is the
    // only one are then both unanswered, and showing a candidate that may have
    // a twin is the ambiguity rule with the ambiguity merely unseen. Neither
    // question is answered by reading, so neither is answered at all.
    if found.primary_uncertain {
        return SessionSourceOutcome::unavailable(SourceUnavailable::Unreadable {
            detail: "part of the local history could not be listed".to_owned(),
        });
    }
    let Some(primary) = found.primary.pop() else {
        return SessionSourceOutcome::unavailable(SourceUnavailable::Missing);
    };
    // The locators the index recorded for this session's own transcript. A
    // rescan after a move records the new place beside the old one, so any
    // recorded spelling of the found file means it is where it was read.
    let recorded: Vec<PathBuf> = request
        .indexed
        .iter()
        .filter_map(IndexedSource::claude_path)
        .map(PathBuf::from)
        .filter(|path| {
            path.file_name().and_then(OsStr::to_str)
                == Some(&format!("{}.jsonl", request.native_session_id))
        })
        .collect();
    if !recorded.is_empty()
        && !recorded
            .iter()
            .any(|path| same_transcript(path, &primary.path))
    {
        return SessionSourceOutcome::unavailable(SourceUnavailable::Moved {
            from: recorded[0].clone(),
            found: primary.path.clone(),
        });
    }
    let mut budget = Budget::new(request.limits);
    let read = match read_claude_file(
        &primary,
        checkpoint_for(request, &primary.path),
        &mut budget,
        request.cancel,
    ) {
        Ok(read) => read,
        Err(failure) => {
            return SessionSourceOutcome::unavailable(primary_failure(failure, &budget));
        }
    };
    let mut loaded = LoadedSession {
        native_session_id: request.native_session_id.to_owned(),
        host: Host::Claude,
        generation: read.generation,
        sources: vec![ReadSource {
            path: primary.path.clone(),
            role: SourceRole::Primary,
            bytes: read.bytes,
        }],
        records: read.records,
        other_lines: read.other_lines,
        dropped_records: read.dropped_records,
        gaps: Vec::new(),
    };
    if found.sidechain_uncertain {
        loaded.gaps.push(SourceGap::DiscoveryIncomplete);
    }
    if let Some((line, reason)) = read.stopped {
        loaded.gaps.push(SourceGap::Stopped {
            path: primary.path,
            line,
            reason,
        });
    }
    // Every subagent transcript the index read, matched against the ones that
    // are there now. A file the index read and this read cannot see is inside
    // the saved measurements and outside the text, which the view has to be
    // told; it is named, not looked for somewhere else.
    //
    // Only a tree that was listed completely can say a file is not in it. When
    // it could not be, the gap above already says the set may be incomplete,
    // and calling an unseen file a lost one would claim more than was seen.
    let observed: BTreeSet<&str> = found
        .sidechains
        .iter()
        .filter_map(|file| sidechain_key(&file.path, request.native_session_id))
        .collect();
    let mut absent: BTreeMap<&str, &Path> = BTreeMap::new();
    for recorded in request
        .indexed
        .iter()
        .filter_map(IndexedSource::claude_path)
    {
        let path = Path::new(recorded);
        if let Some(key) = sidechain_key(path, request.native_session_id)
            && !observed.contains(key)
        {
            // Several recorded spellings of one file are one absent file.
            absent.entry(key).or_insert(path);
        }
    }
    if !found.sidechain_uncertain {
        for path in absent.into_values() {
            loaded.gaps.push(SourceGap::Missing {
                path: path.to_path_buf(),
            });
        }
    }
    for file in found.sidechains {
        match read_claude_file(
            &file,
            checkpoint_for(request, &file.path),
            &mut budget,
            request.cancel,
        ) {
            Ok(read) => {
                loaded.generation = loaded.generation.and(read.generation);
                loaded.sources.push(ReadSource {
                    path: file.path.clone(),
                    role: SourceRole::Sidechain,
                    bytes: read.bytes,
                });
                loaded.records.extend(read.records);
                loaded.other_lines += read.other_lines;
                loaded.dropped_records += read.dropped_records;
                if let Some((line, reason)) = read.stopped {
                    loaded.gaps.push(SourceGap::Stopped {
                        path: file.path,
                        line,
                        reason,
                    });
                }
            }
            Err(ReadFailure::Cancelled) => {
                return SessionSourceOutcome::unavailable(SourceUnavailable::Cancelled);
            }
            // A ceiling belongs to the session, not to the file that happened
            // to reach it: the whole session is too large to show.
            Err(ReadFailure::TooLarge(limit)) => {
                return SessionSourceOutcome::unavailable(budget.too_large(limit));
            }
            // A subagent file that cannot be read leaves the session usable;
            // the gap is named so the view never reads as the whole session.
            Err(ReadFailure::Replaced(basis)) => loaded.gaps.push(SourceGap::Replaced {
                path: file.path,
                basis,
            }),
            Err(ReadFailure::Unreadable(_)) => {
                loaded.gaps.push(SourceGap::Unreadable { path: file.path })
            }
        }
    }
    if cancelled(request.cancel) {
        return SessionSourceOutcome::unavailable(SourceUnavailable::Cancelled);
    }
    SessionSourceOutcome::Loaded(Box::new(loaded))
}

/// A failure reading the session's own transcript leaves nothing to show.
fn primary_failure(failure: ReadFailure, budget: &Budget) -> SourceUnavailable {
    match failure {
        ReadFailure::Replaced(basis) => SourceUnavailable::Replaced { basis },
        ReadFailure::Unreadable(detail) => SourceUnavailable::Unreadable { detail },
        ReadFailure::Cancelled => SourceUnavailable::Cancelled,
        ReadFailure::TooLarge(limit) => budget.too_large(limit),
    }
}

/// The checkpoint the index recorded for this exact file.
fn checkpoint_for<'a>(
    request: &'a SessionSourceRequest<'_>,
    path: &Path,
) -> Option<&'a NativeCheckpoint> {
    request
        .indexed
        .iter()
        .find(|source| {
            source
                .claude_path()
                .is_some_and(|recorded| same_transcript(Path::new(recorded), path))
        })
        .and_then(|source| source.checkpoint.as_ref())
}

// A seam this crate's own tests use to act on a file in the one window that
// matters: after it has been observed and before that observation is validated
// again. It exists so the refusal of a changed file can be proven by
// construction rather than by winning a race with a writer.
//
// In any build that is not this crate's test build it is an empty function
// with no state behind it, so nothing about the contract, the behaviour or the
// surface of this module depends on it.
#[cfg(test)]
type ActOnObserved = std::cell::RefCell<Option<Box<dyn Fn(&Path)>>>;

#[cfg(test)]
thread_local! {
    static OBSERVED: ActOnObserved = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn observed_for_test(path: &Path) {
    OBSERVED.with(|acted| {
        let acted = acted.borrow();
        if let Some(act) = acted.as_ref() {
            act(path);
        }
    });
}

#[cfg(not(test))]
#[inline(always)]
fn observed_for_test(_path: &Path) {}

// The same kind of seam for the exact-detail stream: called after each line is
// parsed, so a cancel can be landed after the last parse boundary by
// construction. Empty outside this crate's test build.
#[cfg(test)]
type ActOnParsed = std::cell::RefCell<Option<Box<dyn Fn(Option<&CancelToken>)>>>;

#[cfg(test)]
thread_local! {
    static PARSED: ActOnParsed = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn parsed_for_test(cancel: Option<&CancelToken>) {
    PARSED.with(|acted| {
        if let Some(act) = acted.borrow().as_ref() {
            act(cancel);
        }
    });
}

#[cfg(not(test))]
#[inline(always)]
fn parsed_for_test(_cancel: Option<&CancelToken>) {}

/// Whether two observations of a file describe the same unchanged file. An
/// inode this platform cannot name proves nothing about identity, so only the
/// length and change time are compared there.
pub(super) fn unchanged(current: &FileIdentity, expected: &FileIdentity) -> bool {
    let same_inode = !(current.known && expected.known)
        || (current.dev == expected.dev && current.ino == expected.ino);
    same_inode && current.len == expected.len && current.ctime_ns == expected.ctime_ns
}

/// Read one Claude file of a session into bounded memory, and prove that what
/// was read is what was verified.
///
/// The file is opened the way the importer opens it — a regular file, no alias
/// followed, still the entry that was observed — and its device, inode,
/// length and change time are captured from the open descriptor. The session's
/// cumulative byte ceiling is charged from that captured length *before*
/// anything is allocated for the file or its generation is proven, so an
/// oversized session costs a stat, not a read.
///
/// Exactly the captured length is then read, in bounded chunks a cancel can
/// interrupt, into memory. A read that ends early means the file shrank under
/// the read. Afterwards both the descriptor and the name are observed again:
/// the descriptor must still describe the file that was read, and the name
/// must still hold it, with the same length and change time. Any mutation
/// across the read — a concurrent append included — makes this a replaced
/// source rather than a transcript to show. An append that had *completed*
/// before the open is not a mutation: it is part of the captured length, and
/// the checkpoint reports it as a session that has grown.
///
/// Only those verified bytes are parsed.
fn read_claude_file(
    file: &ClaudeFile,
    checkpoint: Option<&NativeCheckpoint>,
    budget: &mut Budget,
    cancel: Option<&CancelToken>,
) -> Result<FileOutcome, ReadFailure> {
    if cancelled(cancel) {
        return Err(ReadFailure::Cancelled);
    }
    let unreadable = |error: std::io::Error| {
        ReadFailure::Unreadable(format!("source could not be read: {}", error.kind()))
    };
    let (mut source, identity) = claude_fs::open_source(&file.path, file.observed()).map_err(
        |error| match replaced_since(&file.path, file.observed()) {
            // The name held a different file by the time it was opened: a
            // replacement, which has its own reason, not a read failure.
            true => ReadFailure::Replaced(ResumeBasis::Replaced),
            false => unreadable(error),
        },
    )?;
    // Before any allocation, and before the generation is proven.
    budget
        .take_bytes(identity.len)
        .map_err(ReadFailure::TooLarge)?;
    observed_for_test(&file.path);
    let resume =
        checkpoint::resume_point(checkpoint, &mut source, &identity).map_err(unreadable)?;
    let generation = match resume.basis {
        // Nothing recorded, or an inode this platform cannot name: the file is
        // still the session's, but the read cannot be tied to the measurement.
        ResumeBasis::Fresh | ResumeBasis::Unidentified => GenerationBasis::Unrecorded,
        ResumeBasis::Unchanged => GenerationBasis::Indexed { appended: false },
        ResumeBasis::Appended => GenerationBasis::Indexed { appended: true },
        basis @ (ResumeBasis::Replaced | ResumeBasis::Truncated | ResumeBasis::Rewritten) => {
            return Err(ReadFailure::Replaced(basis));
        }
    };
    source.seek(SeekFrom::Start(0)).map_err(unreadable)?;
    let bytes = snapshot(&mut source, identity.len, cancel, unreadable)?;
    // The descriptor must still describe what was read, and the name must
    // still hold it. Either one having moved makes this a replaced source.
    let after = FileIdentity::of(&source.metadata().map_err(unreadable)?);
    if !unchanged(&after, &identity) {
        return Err(ReadFailure::Replaced(replacement_basis(&after, &identity)));
    }
    let named = FileIdentity::of(&fs::symlink_metadata(&file.path).map_err(unreadable)?);
    if !unchanged(&named, &identity) {
        return Err(ReadFailure::Replaced(replacement_basis(&named, &identity)));
    }
    parse_verified(file, &bytes, generation, budget, cancel)
}

/// How a file that no longer matches its observation differs from it.
fn replacement_basis(current: &FileIdentity, expected: &FileIdentity) -> ResumeBasis {
    if current.known
        && expected.known
        && (current.dev != expected.dev || current.ino != expected.ino)
    {
        ResumeBasis::Replaced
    } else if current.len < expected.len {
        ResumeBasis::Truncated
    } else {
        // Same inode, same or greater length, but written to: an append or an
        // in-place rewrite landed while the file was being read.
        ResumeBasis::Rewritten
    }
}

/// Read exactly `len` bytes from the verified descriptor, in chunks, into
/// memory that is reserved without aborting the process if it cannot be.
fn snapshot(
    source: &mut fs::File,
    len: u64,
    cancel: Option<&CancelToken>,
    unreadable: impl Fn(std::io::Error) -> ReadFailure,
) -> Result<Vec<u8>, ReadFailure> {
    let wanted = usize::try_from(len).map_err(|_| {
        ReadFailure::Unreadable("source is larger than this machine can hold".into())
    })?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(wanted).map_err(|_| {
        ReadFailure::Unreadable("source does not fit in this machine's memory".into())
    })?;
    let mut buffer = [0_u8; READ_CHUNK];
    while bytes.len() < wanted {
        if cancelled(cancel) {
            return Err(ReadFailure::Cancelled);
        }
        let want = READ_CHUNK.min(wanted - bytes.len());
        match source.read(&mut buffer[..want]) {
            // The file is shorter than the length that was verified: it
            // shrank under the read.
            Ok(0) => return Err(ReadFailure::Replaced(ResumeBasis::Truncated)),
            Ok(read) => bytes.extend_from_slice(&buffer[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(unreadable(error)),
        }
    }
    Ok(bytes)
}

/// Parse the verified bytes. The file's own name is the session identity, as
/// it is for the importer, and a line that would have stopped the importer
/// stops the file here at the same place.
fn parse_verified(
    file: &ClaudeFile,
    bytes: &[u8],
    generation: GenerationBasis,
    budget: &mut Budget,
    cancel: Option<&CancelToken>,
) -> Result<FileOutcome, ReadFailure> {
    let context = claude_fs::context(&file.session_id);
    let mut outcome = FileOutcome {
        records: Vec::new(),
        other_lines: 0,
        dropped_records: 0,
        bytes: bytes.len() as u64,
        generation,
        stopped: None,
    };
    let mut lines = 0_u64;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if cancelled(cancel) {
            return Err(ReadFailure::Cancelled);
        }
        // A trailing partial line is not consumed, as the importer leaves it.
        if !line.ends_with(b"\n") {
            break;
        }
        lines += 1;
        let Ok(text) = std::str::from_utf8(line) else {
            outcome.stopped = Some((lines, "transcript is not UTF-8"));
            break;
        };
        if text.trim().is_empty() {
            outcome.other_lines += 1;
            continue;
        }
        match crate::canonical::parse_with_context(text, &context) {
            Ok(Parsed::Record(record)) => {
                let mut record = *record;
                if disagreeing_identity(&record) {
                    outcome.stopped = Some((lines, "native record identity labels disagree"));
                    break;
                }
                // This file is the session, whatever a fork's inherited prefix
                // still calls itself; the importer settles it the same way.
                record.native.session_id = Some(file.session_id.clone());
                record.canonical.native_session_id = Some(file.session_id.clone());
                budget.take_record().map_err(ReadFailure::TooLarge)?;
                outcome.records.push(record);
            }
            Ok(Parsed::Dropped(_)) => outcome.dropped_records += 1,
            // Structural summaries and PR witnesses are the session's history
            // but not its turns; they are counted, never rendered as text.
            Ok(Parsed::Inert | Parsed::StructuralEvent(_) | Parsed::PrLink(_)) => {
                outcome.other_lines += 1
            }
            Err(_) => {
                outcome.stopped = Some((
                    lines,
                    "canonical record does not match the shared stream contract",
                ));
                break;
            }
        }
    }
    if cancelled(cancel) {
        return Err(ReadFailure::Cancelled);
    }
    Ok(outcome)
}

/// Whether the name now holds a different file than the one observed. An
/// unknown inode on either side proves nothing and is not a replacement.
pub(super) fn replaced_since(path: &Path, observed: &fs::Metadata) -> bool {
    fs::symlink_metadata(path).is_ok_and(|current| {
        let current = FileIdentity::of(&current);
        let observed = FileIdentity::of(observed);
        current.known
            && observed.known
            && (current.dev != observed.dev || current.ino != observed.ino)
    })
}

/// The importer's rule: a record's two spellings of its native session must
/// agree and neither may be blank. A record that fails it stops the file.
fn disagreeing_identity(record: &ParsedRecord) -> bool {
    [
        record.native.session_id.as_deref(),
        record.canonical.native_session_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|id| id.trim().is_empty())
        || record
            .canonical
            .native_session_id
            .as_ref()
            .zip(record.native.session_id.as_ref())
            .is_some_and(|(canonical, native)| canonical != native)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(dev: u64, ino: u64, len: u64, ctime_ns: i64) -> FileIdentity {
        FileIdentity {
            dev,
            ino,
            ctime_ns,
            len,
            known: true,
        }
    }

    /// The comparison that decides whether what was read is still what was
    /// verified. It is pinned here rather than only through a race, because
    /// whether a read overlaps a writer is a matter of timing and whether a
    /// changed file is refused must not be.
    #[test]
    fn an_identical_observation_is_the_only_unchanged_one() {
        let opened = observation(1, 2, 4096, 500);
        assert!(unchanged(&opened, &opened));
        for changed in [
            // Another file at the same name.
            observation(1, 3, 4096, 500),
            observation(9, 2, 4096, 500),
            // Written to while it was read: longer, shorter, or rewritten in
            // place with the length preserved.
            observation(1, 2, 8192, 600),
            observation(1, 2, 1024, 600),
            observation(1, 2, 4096, 600),
        ] {
            assert!(!unchanged(&changed, &opened), "{changed:?}");
        }
    }

    /// A platform that cannot name an inode proves nothing by naming one, so
    /// length and change time are all such an observation can be judged by.
    #[test]
    fn an_unnamed_inode_is_judged_by_its_length_and_change_time_alone() {
        let unnamable = FileIdentity {
            known: false,
            ..observation(0, 0, 4096, 500)
        };
        assert!(unchanged(&unnamable, &unnamable));
        assert!(!unchanged(
            &FileIdentity {
                known: false,
                ..observation(0, 0, 4097, 500)
            },
            &unnamable
        ));
        // A known inode compared against an unknown one cannot be called a
        // replacement on identity; only its length and time can disagree.
        assert!(unchanged(&observation(1, 2, 4096, 500), &unnamable));
    }

    #[test]
    fn a_changed_file_is_described_by_how_it_changed() {
        let opened = observation(1, 2, 4096, 500);
        assert_eq!(
            replacement_basis(&observation(1, 3, 4096, 500), &opened),
            ResumeBasis::Replaced
        );
        assert_eq!(
            replacement_basis(&observation(1, 2, 1024, 600), &opened),
            ResumeBasis::Truncated
        );
        for written in [observation(1, 2, 8192, 600), observation(1, 2, 4096, 600)] {
            assert_eq!(replacement_basis(&written, &opened), ResumeBasis::Rewritten);
        }
    }

    /// A line of one synthetic session, written by the test.
    fn line(index: usize, text: &str) -> String {
        format!(
            concat!(
                r#"{{"uuid":"11111111-1111-4111-8111-{:012}","type":"user","#,
                r#""timestamp":"2026-09-07T12:00:{:02}.000Z","#,
                r#""message":{{"role":"user","content":[{{"type":"text","text":"{}"}}]}}}}"#,
                "\n"
            ),
            index, index, text
        )
    }

    fn body(turns: usize, text: &str) -> String {
        (0..turns).map(|index| line(index, text)).collect()
    }

    /// Act on the file in the window between its observation and the
    /// validation of that observation, for the length of one read.
    fn while_in_flight(act: impl Fn(&Path) + 'static, read: impl FnOnce()) {
        OBSERVED.with(|hook| *hook.borrow_mut() = Some(Box::new(act)));
        read();
        OBSERVED.with(|hook| *hook.borrow_mut() = None);
    }

    /// A file written to after it was observed and before that observation is
    /// validated is refused, whatever the writing did to its length. Nothing
    /// about this depends on timing: the write happens inside the window by
    /// construction, which is what a race could only make likely.
    #[test]
    fn a_file_changed_after_it_was_observed_is_refused_before_it_is_returned() {
        for (change, expected) in [
            // Shorter: the read cannot even complete the length it verified.
            ("truncate", ResumeBasis::Truncated),
            // The same length, different bytes: only the change time says so,
            // and only the validation after the read can see it.
            ("overwrite", ResumeBasis::Rewritten),
        ] {
            let temp = tempfile::TempDir::new().unwrap();
            let path = temp
                .path()
                .join("00000000-0000-4000-8000-0000000000aa.jsonl");
            let settled = body(40, "ask");
            fs::write(&path, &settled).unwrap();
            let file = claude_fs::observe(
                path.clone(),
                "00000000-0000-4000-8000-0000000000aa".to_owned(),
                false,
            )
            .unwrap();
            let replacement = match change {
                "truncate" => body(3, "ask"),
                _ => body(40, "AsK"),
            };
            assert_eq!(
                change == "overwrite",
                replacement.len() == settled.len(),
                "{change} must change the length only when it means to"
            );
            let mut outcome = None;
            while_in_flight(
                move |path| {
                    // A plain write through the same name: the file keeps its
                    // inode, as an editor or a host appending would.
                    fs::write(path, &replacement).unwrap();
                },
                || {
                    outcome = Some(read_claude_file(
                        &file,
                        None,
                        &mut Budget::new(SourceLimits::default()),
                        None,
                    ));
                },
            );
            match outcome.unwrap() {
                Err(ReadFailure::Replaced(basis)) => assert_eq!(basis, expected, "{change}"),
                Err(_) => panic!("{change}: refused for the wrong reason"),
                // The type of the refusal is the guarantee: a refusal carries
                // no records, so no part of either generation can be shown.
                Ok(read) => panic!("{change}: returned {} records", read.records.len()),
            }
        }
    }

    /// The same window, left alone: the read returns the file it observed.
    #[test]
    fn a_file_left_alone_inside_that_window_is_returned_whole() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp
            .path()
            .join("00000000-0000-4000-8000-0000000000aa.jsonl");
        fs::write(&path, body(40, "ask")).unwrap();
        let file = claude_fs::observe(
            path.clone(),
            "00000000-0000-4000-8000-0000000000aa".to_owned(),
            false,
        )
        .unwrap();
        let Ok(read) =
            read_claude_file(&file, None, &mut Budget::new(SourceLimits::default()), None)
        else {
            panic!("an untouched file is returned");
        };
        assert_eq!(read.records.len(), 40);
        assert_eq!(read.generation, GenerationBasis::Unrecorded);
    }

    /// A cancel that lands after the last line is parsed — after every check
    /// inside the loop — still returns nothing. The seam lands it there by
    /// construction.
    #[test]
    fn a_cancel_after_the_last_parsed_line_returns_nothing() {
        let home = PathBuf::from("/home/user");
        let native = "00000000-0000-4000-8000-00000000c0de";
        let stream = format!(
            "{}\n{}\n",
            serde_json::json!({
                "type": "session", "host": "codex", "native_session_id": native,
                "conversation_id": format!("codex-{native}"), "source_surface": null,
                "started_at": null, "cwd": null, "git_branch": null, "title": null,
                "path": "/home/user/.codex/sessions/rollout.jsonl", "mtime": 1.0
            }),
            serde_json::json!({
                "uuid": "77770000-7777-4777-8777-000000000000", "type": "user",
                "timestamp": "2026-09-07T12:00:00Z",
                "message": {"role": "user", "content": [{"type": "text", "text": "ask"}]}
            })
        );
        let roots = host_roots(Host::Codex, &home);
        let cancel = CancelToken::new();
        let request = SessionSourceRequest {
            home: &home,
            host: Host::Codex,
            native_session_id: native,
            indexed: &[],
            cancel: Some(&cancel),
            limits: SourceLimits::default(),
        };
        // Left alone, the stream is the session.
        assert!(session_from_stream(&request, &roots, stream.as_bytes()).is_ok());
        let lines = std::rc::Rc::new(std::cell::Cell::new(0));
        let seen = std::rc::Rc::clone(&lines);
        PARSED.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |cancel| {
                seen.set(seen.get() + 1);
                // The record is the second and last line.
                if seen.get() == 2 {
                    cancel.unwrap().cancel();
                }
            }))
        });
        let outcome = session_from_stream(&request, &roots, stream.as_bytes());
        PARSED.with(|hook| *hook.borrow_mut() = None);
        assert_eq!(lines.get(), 2);
        assert_eq!(outcome, Err(SourceUnavailable::Cancelled));
    }

    /// A source is accepted under the host's roots, as spelled or where a
    /// symlinked root led, and nowhere else under the home.
    #[test]
    fn a_reported_source_must_sit_under_an_anchored_host_root() {
        let roots = vec![
            PathBuf::from("/elsewhere/codex-sessions"),
            PathBuf::from("/home/user/.codex/sessions"),
        ];
        for accepted in [
            "/home/user/.codex/sessions/2026/rollout.jsonl",
            "/elsewhere/codex-sessions/2026/rollout.jsonl",
        ] {
            assert!(under_roots(Path::new(accepted), &roots), "{accepted}");
        }
        for refused in [
            "/home/user/.codex/other/rollout.jsonl",
            "/home/user/secret.jsonl",
            "/home/user/.codex/sessions-evil/rollout.jsonl",
            "/home/user/.codex/sessions/../../secret.jsonl",
            ".codex/sessions/rollout.jsonl",
            "/elsewhere/rollout.jsonl",
        ] {
            assert!(!under_roots(Path::new(refused), &roots), "{refused}");
        }
    }

    /// One file that cannot be tied to a measurement leaves the session
    /// untied; otherwise one file that grew makes the session a grown one.
    #[test]
    fn a_sessions_generation_is_the_weakest_of_its_files() {
        let settled = GenerationBasis::Indexed { appended: false };
        let grown = GenerationBasis::Indexed { appended: true };
        assert_eq!(settled.and(settled), settled);
        assert_eq!(settled.and(grown), grown);
        assert_eq!(grown.and(settled), grown);
        for other in [settled, grown, GenerationBasis::Unrecorded] {
            assert_eq!(
                GenerationBasis::Unrecorded.and(other),
                GenerationBasis::Unrecorded
            );
            assert_eq!(
                other.and(GenerationBasis::Unrecorded),
                GenerationBasis::Unrecorded
            );
        }
    }
}
