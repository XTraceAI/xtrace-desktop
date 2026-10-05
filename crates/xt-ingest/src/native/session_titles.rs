//! Reading the **host's own title** for a few visible sessions, on demand,
//! from their original local sources.
//!
//! The index never stores a title taken from a transcript. A list that wants
//! to show the name a host gave a session reads it here, for the rows it is
//! showing, and drops it when the view goes: nothing is written, cached or
//! logged, and the source files are opened for reading only.
//!
//! This is a title read, not a transcript read. Each file is streamed once in
//! bounded chunks; only a line that could carry a title is ever parsed, and no
//! record, prompt or other text is kept. The rules that decide what counts as
//! a title are the hosts' own:
//!
//! - **Claude Code.** The last valid `custom-title` (`customTitle`, a user's
//!   rename) outranks the last valid `ai-title` (`aiTitle`, generated),
//!   whatever order they were written in: the client keeps writing a stale
//!   generated title after a rename. Within each kind the last valid record
//!   wins. A sidechain record, a record naming another session, and a value
//!   that is not a nonblank string are ignored.
//! - **Codex.** The last valid `event_msg`/`thread_name_updated` of the
//!   verified rollout. Only when that rollout names no thread, the last
//!   matching `id`/`thread_name` row in the bounded tail of the host's
//!   `session_index.jsonl` sidecar, read once per batch. A rollout rename
//!   is never overridden by the sidecar, whose rows carry no chronology this
//!   read could weigh against it, and the sidecar never names a session whose
//!   rollout could not be verified.
//! - **Codex, paginated or inherited.** A thread whose history spans several
//!   rollout files (`rollout-<ts>-<thread>.jsonl`, then continuations named
//!   `rollout-<ts>-<thread>_<rollout>.jsonl`) is named by the sidecar alone.
//!   Only the opening `session_meta` line of each indexed file is read, to
//!   prove it is this thread; its body is never read, so a rename in one
//!   segment — which a later sibling may supersede — is never shown as the
//!   thread's name. A `history_base` names the rollout a history continues
//!   from, never the thread: the sidecar is asked only for this thread. A
//!   spawned thread's header names its parent in `session_id`; that is
//!   accepted only as the parent its own typed `thread_spawn` names.
//! - **Cursor and anything else.** No host title exists; nothing is read. A
//!   title derived from a prompt is never produced for any host.
//!
//! Every source is the one the index recorded, reached without following an
//! alias, contained in its host's history root, and the same file — device,
//! inode, length and change time — before and after it is streamed. A Claude
//! transcript must also still hold the generation its checkpoint recorded. A
//! missing, moved, ambiguous, replaced, oversized or cancelled source, one
//! with a complete line too long to inspect, a spent batch budget or the batch
//! deadline gives that row no title, which leaves its identifier in place; a
//! wrong session's title is never returned.

use super::checkpoint::{self, FileIdentity, ResumeBasis};
use super::readers_cli::CancelToken;
use super::session_source::{
    IndexedSource, host_roots, identifier_error, replaced_since, unchanged, under_roots,
};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};
use xt_store::{
    Host, SessionSource, Store,
    batch::{LocatorRows, escape_like},
};

/// The most sessions one batch may name: one Sessions page.
pub const MAX_TITLE_SESSIONS: usize = 50;

/// The largest single source a title is read from. A session beyond it keeps
/// its identifier rather than a title taken from part of it.
pub const MAX_TITLE_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// The most bytes one batch may charge, over every source it reads. A Claude
/// transcript with a checkpoint is charged twice its length: its recorded
/// generation is proven by reading up to the checkpoint before it is streamed.
pub const MAX_TITLE_BATCH_BYTES: u64 = 512 * 1024 * 1024;

/// The longest line inspected. A Codex rollout's opening `session_meta` can
/// hold long instructions; a title line is a few hundred bytes. A longer
/// complete line is never partly parsed, and because it could hold the latest
/// title it leaves its whole source untitled.
pub const MAX_TITLE_LINE_BYTES: usize = 1024 * 1024;

/// The longest title returned, in characters, as the Codex reader bounds a
/// host name. A longer one is cut, never dropped.
pub const MAX_TITLE_CHARS: usize = 200;

/// How long one batch may read before every row not yet read keeps its
/// identifier. The deadline is cooperative: it is heard between reads of a
/// bounded chunk, so one slow read from the file system can still carry a
/// batch past it.
pub const TITLE_DEADLINE: Duration = Duration::from_secs(5);

/// How much of the end of Codex's `session_index.jsonl` is read, as the Codex
/// reader bounds it: the sidecar is append-ordered and grows forever.
pub const SIDECAR_TAIL_BYTES: u64 = 4 * 1024 * 1024;

/// How many lines of a rollout may precede its `session_meta`, as the Codex
/// reader bounds its own header probe.
const HEADER_LINES: u64 = 200;

/// The most rollout files the index may record for one Codex thread before a
/// title read declines it: a paginated history has a root and a few
/// continuations, and each one's header is read to prove it.
pub const MAX_CODEX_LOCATORS: usize = 8;

/// Bytes read per call while a rollout's opening `session_meta` line is
/// probed, kept small so what is read past that line's end is little. Those
/// bytes are never parsed.
const HEADER_CHUNK: usize = 16 * 1024;

/// Bytes read per call while a source is streamed; a cancel or the deadline
/// lands between chunks.
const READ_CHUNK: usize = 256 * 1024;

/// The ceilings one batch observes. Defaults are the constants above; tests
/// lower them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleLimits {
    pub max_file_bytes: u64,
    pub max_batch_bytes: u64,
    pub max_line_bytes: usize,
    pub deadline: Duration,
}

impl Default for TitleLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: MAX_TITLE_FILE_BYTES,
            max_batch_bytes: MAX_TITLE_BATCH_BYTES,
            max_line_bytes: MAX_TITLE_LINE_BYTES,
            deadline: TITLE_DEADLINE,
        }
    }
}

/// One session to title: what the index recorded for it, resolved by the
/// caller under its own lock. Nothing here names a path the caller chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleTarget {
    pub host: Host,
    pub native_session_id: String,
    /// The locators [`title_sources`] returned for this session.
    pub locators: Vec<IndexedSource>,
}

/// Where a returned title came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleSource {
    /// Claude Code's `custom-title`: the user renamed the session.
    ClaudeRename,
    /// Claude Code's `ai-title`, with no rename present.
    ClaudeGenerated,
    /// Codex's `thread_name_updated` in the session's own rollout.
    CodexRollout,
    /// Codex's `session_index.jsonl`, for a verified rollout that names no
    /// thread itself, or for a paginated or inherited history whose indexed
    /// rollout headers were verified: that history's only title here.
    CodexSidecar,
}

/// Why a row has no title. Each keeps the row's identifier; none is shown or
/// logged by this module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Untitled {
    /// The source was read whole and names no valid host title.
    NoTitle,
    /// This host writes no title this read may use.
    UnsupportedHost,
    /// The identifier could not name one session's file.
    InvalidIdentifier,
    /// The index recorded no source for this session.
    NotIndexed,
    /// No recorded source is there any more.
    Missing,
    /// A recorded source is not a file directly inside its host's history
    /// root, or is reached through an alias.
    Outside,
    /// More than one distinct file claims this session.
    Ambiguous,
    /// The source is not the generation the index recorded, or it changed
    /// while it was read.
    Replaced,
    /// The source could not be read.
    Unreadable,
    /// The rollout's own header names another session, or names none.
    IdentityMismatch,
    /// The rollout declares a paginated or inherited history, but not in its
    /// opening line, where every segment of such a history declares it.
    UnsupportedHistory,
    /// The index recorded more rollout files for this thread than a title
    /// read verifies.
    TooManySources,
    /// The source is larger than a title read may stream.
    TooLarge,
    /// A complete line is longer than a title read inspects; it could hold a
    /// later title than any line that was read.
    LineTooLong,
    /// Earlier rows spent the batch's byte budget.
    Budget,
    /// The batch reached its deadline before this row was read.
    Deadline,
    /// The caller cancelled; no row of the batch has a title.
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TitleOutcome {
    Titled { title: String, source: TitleSource },
    Untitled(Untitled),
}

impl TitleOutcome {
    pub fn title(&self) -> Option<&str> {
        match self {
            Self::Titled { title, .. } => Some(title),
            Self::Untitled(_) => None,
        }
    }
}

/// One outcome per target, in target order, and how many source bytes the
/// batch actually read. Dropping this value is the whole of its lifecycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleBatch {
    pub outcomes: Vec<TitleOutcome>,
    pub bytes_read: u64,
}

/// The locators the index recorded that a title read may use for one session.
///
/// Claude: the session's own transcripts (never a subagent file). Codex: the
/// rollouts recorded under this exact native identity — its root and any
/// `<thread>_<rollout>` continuation — at most one more than
/// [`MAX_CODEX_LOCATORS`], so that a thread with too many is seen as such.
/// The index is asked for at most [`xt_store::batch::MAX_LOCATOR_ROWS`] keys
/// the patterns match before names are checked exactly; a thread whose
/// patterns match more has none, and keeps its identifier, because the exact
/// set could not be known from a bounded prefix. Other hosts: none.
pub fn title_sources(
    store: &Store,
    host: Host,
    native_session_id: &str,
) -> xt_store::Result<Vec<IndexedSource>> {
    if identifier_error(native_session_id).is_some() {
        return Ok(Vec::new());
    }
    match host {
        // The session's own transcript only: its subagent files carry no
        // title of the session, so their checkpoints are not even looked up.
        Host::Claude => {
            let pattern = format!("claude:%/{}.jsonl", escape_like(native_session_id));
            store
                .source_cursors_like(SessionSource::Transcript, &[&pattern])?
                .into_iter()
                .filter(|cursor| {
                    cursor
                        .cursor_key
                        .strip_prefix("claude:")
                        .is_some_and(|path| primary_name(Path::new(path), native_session_id))
                })
                .map(|cursor| {
                    Ok(IndexedSource {
                        checkpoint: store
                            .native_checkpoint(SessionSource::Transcript, &cursor.cursor_key)?,
                        locator: cursor.cursor_key,
                    })
                })
                .collect()
        }
        Host::Codex => {
            let escaped = escape_like(native_session_id);
            let root = format!("codex:%-{escaped}.jsonl");
            let continuation = format!("codex:%-{escaped}\\_%.jsonl");
            let LocatorRows::Complete(rows) = store
                .source_cursors_like_bounded(SessionSource::ReadersCli, &[&root, &continuation])?
            else {
                return Ok(Vec::new());
            };
            Ok(rows
                .into_iter()
                .filter(|cursor| {
                    codex_path(&cursor.cursor_key).is_some_and(|path| {
                        codex_segment(Path::new(path), native_session_id).is_some()
                    })
                })
                .take(MAX_CODEX_LOCATORS + 1)
                .map(|cursor| IndexedSource {
                    checkpoint: None,
                    locator: cursor.cursor_key,
                })
                .collect())
        }
        Host::Cursor | Host::Other => Ok(Vec::new()),
    }
}

pub(super) fn codex_path(locator: &str) -> Option<&str> {
    locator.strip_prefix("codex:")
}

/// A Claude session's own transcript is named after it.
fn primary_name(path: &Path, native: &str) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some(&format!("{native}.jsonl"))
}

/// A Codex rollout is named `rollout-<timestamp>-<native>.jsonl`.
fn rollout_name(path: &Path, native: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.starts_with("rollout-") && name.ends_with(&format!("-{native}.jsonl"))
        })
}

/// Which file of a Codex thread a rollout name says it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Segment {
    /// `rollout-<timestamp>-<thread>.jsonl`: the thread's own rollout, whose
    /// immutable rollout identity is the thread's.
    Root,
    /// `rollout-<timestamp>-<thread>_<rollout>.jsonl`: a continuation, with
    /// its own immutable rollout identity. Both halves are UUIDs.
    Continuation(String),
}

/// The segment a rollout name makes it of the thread `native`, if any.
pub(super) fn codex_segment(path: &Path, native: &str) -> Option<Segment> {
    if rollout_name(path, native) {
        return Some(Segment::Root);
    }
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    let (head, rollout) = stem.rsplit_once('_')?;
    let thread = head.strip_suffix(native)?.strip_suffix('-')?;
    (!thread.is_empty() && uuid_shaped(native) && uuid_shaped(rollout))
        .then(|| Segment::Continuation(rollout.to_owned()))
}

/// `8-4-4-4-12` hexadecimal digits, as Codex spells a thread or rollout.
pub(super) fn uuid_shaped(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(group, len)| group.len() == len && group.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// A full Codex thread identity as Codex writes one: a lowercase UUID.
pub(super) fn full_thread_id(text: &str) -> bool {
    uuid_shaped(text) && !text.bytes().any(|byte| byte.is_ascii_uppercase())
}

/// What a Codex `session_meta` payload's typed `source` says about a spawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TypedSpawn<'a> {
    /// No `source.subagent.thread_spawn`: a person's session, `exec`, or a
    /// `guardian`/`review` subagent.
    None,
    /// A `thread_spawn` beside another key, or whose parent is not one full
    /// thread identity.
    Malformed,
    /// A `thread_spawn` naming the thread itself.
    SelfSpawn,
    /// A `thread_spawn` naming this distinct full parent. Its
    /// `payload.session_id` has not been looked at.
    Parent(&'a str),
}

/// Read `payload.source` as a spawn of the thread `native`: only
/// `{"subagent": {"thread_spawn": {"parent_thread_id": <thread>}}}`, with
/// nothing beside either key, names a parent. No other field — `history_base`,
/// depth, role, nickname or task path — is consulted.
pub(super) fn typed_spawn<'a>(payload: &'a Map<String, Value>, native: &str) -> TypedSpawn<'a> {
    let Some(source) = payload.get("source").and_then(Value::as_object) else {
        return TypedSpawn::None;
    };
    let Some((subagent, spawn)) = source
        .get("subagent")
        .and_then(Value::as_object)
        .and_then(|subagent| subagent.get("thread_spawn").map(|spawn| (subagent, spawn)))
    else {
        return TypedSpawn::None;
    };
    if source.len() != 1 || subagent.len() != 1 {
        return TypedSpawn::Malformed;
    }
    match spawn
        .as_object()
        .and_then(|spawn| spawn.get("parent_thread_id"))
        .and_then(Value::as_str)
        .filter(|parent| full_thread_id(parent))
    {
        None => TypedSpawn::Malformed,
        Some(parent) if parent == native => TypedSpawn::SelfSpawn,
        Some(parent) => TypedSpawn::Parent(parent),
    }
}

/// Whether a spawned thread's `payload.session_id` is present and names
/// exactly the parent its typed spawn names. Every observed spawned header
/// carries its parent there; it corroborates the typed parent and never
/// supplies one.
pub(super) fn corroborated(payload: &Map<String, Value>, parent: &str) -> bool {
    payload.get("session_id").and_then(Value::as_str) == Some(parent)
}

/// Read the host title of each target, in order, within one budget and one
/// deadline. A cancel at any point returns no title for any row.
pub fn read_titles(
    home: &Path,
    targets: &[TitleTarget],
    limits: TitleLimits,
    cancel: Option<&CancelToken>,
) -> TitleBatch {
    let mut batch = Batch::new(limits, cancel);
    let mut outcomes = Vec::with_capacity(targets.len());
    // Codex rows whose verified rollout names no thread, by position.
    let mut sidecar_rows: Vec<(usize, &str)> = Vec::new();
    for (position, target) in targets.iter().enumerate() {
        if let Some(why) = batch.stop.stopped() {
            outcomes.push(TitleOutcome::Untitled(why));
            continue;
        }
        let outcome = match target.host {
            Host::Claude => batch.claude(home, target),
            Host::Codex => match batch.codex(home, target) {
                Ok(Some(title)) => TitleOutcome::Titled {
                    title,
                    source: TitleSource::CodexRollout,
                },
                // A verified rollout that names no thread, or a verified
                // paginated or inherited history: the sidecar may name it.
                Ok(None) => {
                    sidecar_rows.push((position, &target.native_session_id));
                    TitleOutcome::Untitled(Untitled::NoTitle)
                }
                Err(why) => TitleOutcome::Untitled(why),
            },
            Host::Cursor | Host::Other => TitleOutcome::Untitled(Untitled::UnsupportedHost),
        };
        outcomes.push(outcome);
    }
    if !sidecar_rows.is_empty() && batch.stop.stopped().is_none() {
        let wanted: Vec<&str> = sidecar_rows.iter().map(|(_, native)| *native).collect();
        let names = batch.sidecar(home, &wanted);
        for (position, native) in sidecar_rows {
            if let Some(title) = names.get(native) {
                outcomes[position] = TitleOutcome::Titled {
                    title: title.clone(),
                    source: TitleSource::CodexSidecar,
                };
            }
        }
    }
    // A cancel that landed at any point, the last read included, returns
    // nothing: the caller was told nothing would come.
    if batch.stop.cancelled() {
        outcomes = vec![TitleOutcome::Untitled(Untitled::Cancelled); targets.len()];
    }
    TitleBatch {
        outcomes,
        bytes_read: batch.bytes_read,
    }
}

struct Stop<'a> {
    cancel: Option<&'a CancelToken>,
    deadline: Instant,
}

impl Stop<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.is_some_and(CancelToken::is_cancelled)
    }

    fn stopped(&self) -> Option<Untitled> {
        if self.cancelled() {
            Some(Untitled::Cancelled)
        } else if Instant::now() >= self.deadline {
            Some(Untitled::Deadline)
        } else {
            None
        }
    }
}

/// One bounded read of several sources: a byte budget, a deadline and a
/// cancel, shared by every source it reads.
pub(super) struct Batch<'a> {
    stop: Stop<'a>,
    limits: TitleLimits,
    /// Bytes charged against the budget before each read.
    charged: u64,
    /// Bytes actually read from sources.
    bytes_read: u64,
}

/// A recorded source that passed containment and was observed as a regular
/// file, with the checkpoint recorded for it.
pub(super) struct Candidate<'a> {
    pub(super) path: PathBuf,
    observed: fs::Metadata,
    checkpoint: Option<&'a xt_store::batch::NativeCheckpoint>,
}

impl<'a> Batch<'a> {
    pub(super) fn new(limits: TitleLimits, cancel: Option<&'a CancelToken>) -> Self {
        Self {
            stop: Stop {
                cancel,
                deadline: Instant::now() + limits.deadline,
            },
            limits,
            charged: 0,
            bytes_read: 0,
        }
    }

    /// Why nothing more may be read: a cancel or the deadline.
    pub(super) fn stopped(&self) -> Option<Untitled> {
        self.stop.stopped()
    }

    /// Whether the caller cancelled at any point so far.
    pub(super) fn cancelled(&self) -> bool {
        self.stop.cancelled()
    }

    /// Bytes actually read from sources so far.
    pub(super) fn bytes_read(&self) -> u64 {
        self.bytes_read
    }

    /// What is left of the budget and the deadline, for further work that
    /// shares this call's limits.
    pub(super) fn remaining(&self) -> (u64, Duration) {
        (
            self.limits.max_batch_bytes.saturating_sub(self.charged),
            self.stop.deadline.saturating_duration_since(Instant::now()),
        )
    }

    /// Charge a read before anything is allocated for it.
    fn charge(&mut self, bytes: u64) -> Result<(), Untitled> {
        let total = self.charged.saturating_add(bytes);
        if total > self.limits.max_batch_bytes {
            return Err(Untitled::Budget);
        }
        self.charged = total;
        Ok(())
    }

    fn claude(&mut self, home: &Path, target: &TitleTarget) -> TitleOutcome {
        let native = target.native_session_id.as_str();
        let found = select(
            target
                .locators
                .iter()
                .filter_map(|source| Some((source.claude_path()?, source))),
            native,
            &[home.join(".claude/projects")],
            |path, roots| claude_contained(path, roots, native),
        );
        let candidate = match found {
            Ok(candidate) => candidate,
            Err(why) => return TitleOutcome::Untitled(why),
        };
        let mut titles = ClaudeTitles::new(native);
        match self.stream(&candidate, Proof::Checkpoint, &mut |line| titles.line(line)) {
            Ok(()) => titles.outcome(),
            Err(why) => TitleOutcome::Untitled(why),
        }
    }

    /// The rollout's own thread name; `Ok(None)` when the verified rollout
    /// names none, or the thread's history is paginated or inherited, so only
    /// the sidecar may.
    ///
    /// Every recorded rollout's opening line is read first. A rollout whose
    /// header opens a flat history is streamed whole, as before, only when it
    /// is the one locator the index recorded for this thread: beside any other
    /// recorded file — live, gone, an alias or outside the root — its name
    /// may be one a later segment superseded, so the row keeps its identifier
    /// and not even the sidecar names it. Otherwise
    /// each indexed file must open with this thread's paginated or inherited
    /// `session_meta`, each must be a distinct rollout, at most one may be the
    /// history's original, and no recorded file may be an alias or outside
    /// the root; nothing past those headers is read. A recorded file that is
    /// gone is tolerated only because another was verified.
    fn codex(&mut self, home: &Path, target: &TitleTarget) -> Result<Option<String>, Untitled> {
        let native = target.native_session_id.as_str();
        if target.locators.len() > MAX_CODEX_LOCATORS {
            return Err(Untitled::TooManySources);
        }
        let (candidates, outside) = gather(
            target
                .locators
                .iter()
                .filter_map(|source| Some((codex_path(&source.locator)?, source))),
            native,
            &host_roots(Host::Codex, home),
            |path, roots| codex_contained(path, roots, native),
        )?;
        // The whole recorded set, not only what is still there: a gone or
        // unsafe locator may be the segment that renamed the thread last.
        let sole = target.locators.len() == 1 && candidates.len() == 1 && !outside;
        let mut rollouts = std::collections::BTreeSet::new();
        let mut originals = 0_usize;
        for candidate in &candidates {
            if let Some(why) = self.stop.stopped() {
                return Err(why);
            }
            let segment = codex_segment(&candidate.path, native).ok_or(Untitled::Outside)?;
            let header = self.header(candidate, native)?;
            match (header, &segment) {
                (Header::Paged { original }, _) => originals += usize::from(original),
                // One rollout of a flat history: its own name, else the
                // sidecar, exactly as before.
                (Header::Flat | Header::NotFirst, Segment::Root) if sole => {
                    let mut rollout = CodexRollout::new(native);
                    self.stream(candidate, Proof::None, &mut |line| rollout.line(line))?;
                    return rollout.outcome();
                }
                // A flat rollout of the thread beside another recorded file
                // claiming it, whether or not that file is still there.
                (Header::Flat | Header::NotFirst, Segment::Root) => {
                    return Err(Untitled::Ambiguous);
                }
                // A continuation's name over a header that is not one.
                (Header::Flat | Header::NotFirst, Segment::Continuation(_)) => {
                    return Err(Untitled::IdentityMismatch);
                }
            }
            let rollout = match segment {
                Segment::Root => native.to_owned(),
                Segment::Continuation(rollout) => rollout,
            };
            if !rollouts.insert(rollout) {
                return Err(Untitled::Ambiguous);
            }
        }
        if originals > 1 {
            return Err(Untitled::Ambiguous);
        }
        // A recorded file of this history that could not be looked at safely
        // may be the part that disagrees; the ones that could are not enough.
        if outside {
            return Err(Untitled::Outside);
        }
        Ok(None)
    }

    /// Read only a rollout's opening line and say what history it opens.
    fn header(&mut self, candidate: &Candidate<'_>, native: &str) -> Result<Header, Untitled> {
        self.opening_line(candidate, &mut |line| codex_header(line, native))?
    }

    /// Read only a source's opening line and hand it to `parse`; nothing past
    /// it is parsed.
    ///
    /// The line must be complete and within the line bound. The file must be
    /// the one observed — device, inode, length and change time — when it is
    /// opened, after the line is read, and by name after that; any change,
    /// an append included, refuses it.
    pub(super) fn opening_line<T>(
        &mut self,
        candidate: &Candidate<'_>,
        parse: &mut dyn FnMut(&[u8]) -> T,
    ) -> Result<T, Untitled> {
        let (mut source, identity) =
            open_regular(&candidate.path, &candidate.observed).map_err(|_| {
                if replaced_since(&candidate.path, &candidate.observed) {
                    Untitled::Replaced
                } else {
                    Untitled::Unreadable
                }
            })?;
        let observed = FileIdentity::of(&candidate.observed);
        if !unchanged(&identity, &observed) {
            return Err(Untitled::Replaced);
        }
        let bound = self.limits.max_line_bytes;
        let limit = identity.len.min(bound as u64 + 1);
        self.charge(limit)?;
        let mut first: Option<T> = None;
        let read = scan_lines(
            &mut source,
            limit,
            bound,
            HEADER_CHUNK,
            &self.stop,
            &mut |line| {
                first = Some(parse(line));
                Flow::Stop
            },
        );
        self.bytes_read += match &read {
            Ok(bytes) | Err((_, bytes)) => *bytes,
        };
        read.map_err(|(why, _)| why)?;
        let after = source.metadata().map_err(|_| Untitled::Unreadable)?;
        let named = fs::symlink_metadata(&candidate.path).map_err(|_| Untitled::Replaced)?;
        if !unchanged(&FileIdentity::of(&after), &observed)
            || !unchanged(&FileIdentity::of(&named), &observed)
        {
            return Err(Untitled::Replaced);
        }
        match first {
            Some(parsed) => Ok(parsed),
            // No newline within the bound: the line is longer than it.
            None if identity.len > bound as u64 => Err(Untitled::LineTooLong),
            // The file is one torn line, or empty: it names no thread.
            None => Err(Untitled::IdentityMismatch),
        }
    }

    /// Stream one verified source line by line, and prove afterwards that the
    /// bytes streamed are the file that was observed.
    pub(super) fn stream(
        &mut self,
        candidate: &Candidate<'_>,
        proof: Proof,
        on_line: &mut dyn FnMut(&[u8]) -> Flow,
    ) -> Result<(), Untitled> {
        let (mut source, identity) =
            open_regular(&candidate.path, &candidate.observed).map_err(|_| {
                if replaced_since(&candidate.path, &candidate.observed) {
                    Untitled::Replaced
                } else {
                    Untitled::Unreadable
                }
            })?;
        if identity.len > self.limits.max_file_bytes {
            return Err(Untitled::TooLarge);
        }
        let proving = proof == Proof::Checkpoint && candidate.checkpoint.is_some();
        self.charge(identity.len.saturating_mul(if proving { 2 } else { 1 }))?;
        if proof == Proof::Checkpoint {
            let resume = prove(candidate.checkpoint, &mut source, &identity, &mut || {
                self.stop.stopped()
            })?;
            self.bytes_read += match resume.basis {
                // Only the recorded tail was read to prove it.
                ResumeBasis::Unchanged if !resume.refresh => resume.tail.len() as u64,
                ResumeBasis::Unchanged | ResumeBasis::Appended => resume.start,
                _ => 0,
            };
            if matches!(
                resume.basis,
                ResumeBasis::Replaced | ResumeBasis::Truncated | ResumeBasis::Rewritten
            ) {
                return Err(Untitled::Replaced);
            }
            source
                .seek(SeekFrom::Start(0))
                .map_err(|_| Untitled::Unreadable)?;
        }
        let read = scan_lines(
            &mut source,
            identity.len,
            self.limits.max_line_bytes,
            READ_CHUNK,
            &self.stop,
            on_line,
        );
        self.bytes_read += match &read {
            Ok(bytes) | Err((_, bytes)) => *bytes,
        };
        read.map_err(|(why, _)| why)?;
        // The descriptor must still describe what was read, and the name must
        // still hold it; an append during the read is a change like any other.
        let after = source.metadata().map_err(|_| Untitled::Unreadable)?;
        let named = fs::symlink_metadata(&candidate.path).map_err(|_| Untitled::Replaced)?;
        if !unchanged(&FileIdentity::of(&after), &identity)
            || !unchanged(&FileIdentity::of(&named), &identity)
        {
            return Err(Untitled::Replaced);
        }
        Ok(())
    }

    /// The last valid thread name of each wanted session in the sidecar's
    /// bounded tail. Any failure, or a sidecar that changed while it was read,
    /// names nothing.
    fn sidecar(&mut self, home: &Path, wanted: &[&str]) -> BTreeMap<String, String> {
        let path = home.join(".codex/session_index.jsonl");
        let Ok(observed) = fs::symlink_metadata(&path) else {
            return BTreeMap::new();
        };
        if !observed.is_file() {
            return BTreeMap::new();
        }
        let Ok((mut source, identity)) = open_regular(&path, &observed) else {
            return BTreeMap::new();
        };
        let start = identity.len.saturating_sub(SIDECAR_TAIL_BYTES);
        let length = identity.len - start;
        if self.charge(length).is_err() || source.seek(SeekFrom::Start(start)).is_err() {
            return BTreeMap::new();
        }
        let mut names = BTreeMap::new();
        // A tail that starts inside a row begins with the rest of that row;
        // it is not a row of its own.
        let mut first = start > 0;
        let read = scan_lines(
            &mut source,
            length,
            self.limits.max_line_bytes,
            READ_CHUNK,
            &self.stop,
            &mut |line| {
                if std::mem::take(&mut first) {
                    return Flow::Continue;
                }
                if let Some((id, name)) = sidecar_row(line, wanted) {
                    names.insert(id, name);
                }
                Flow::Continue
            },
        );
        self.bytes_read += match &read {
            Ok(bytes) | Err((_, bytes)) => *bytes,
        };
        let intact = read.is_ok()
            && source
                .metadata()
                .is_ok_and(|after| unchanged(&FileIdentity::of(&after), &identity))
            && fs::symlink_metadata(&path)
                .is_ok_and(|named| unchanged(&FileIdentity::of(&named), &identity));
        if intact { names } else { BTreeMap::new() }
    }
}

/// Why a checkpoint proof ended before it decided.
enum ProofHalt {
    Stopped(Untitled),
    Unreadable,
}

impl From<std::io::Error> for ProofHalt {
    fn from(_: std::io::Error) -> Self {
        Self::Unreadable
    }
}

/// Prove `source` against its checkpoint, hearing `stopped` before and after
/// every read the proof makes: a cancel or the deadline ends a long prefix
/// proof between chunks rather than after it.
fn prove(
    recorded: Option<&xt_store::batch::NativeCheckpoint>,
    source: &mut fs::File,
    identity: &FileIdentity,
    stopped: &mut dyn FnMut() -> Option<Untitled>,
) -> Result<checkpoint::Resume, Untitled> {
    checkpoint::resume_point_checked(recorded, source, identity, &mut || {
        stopped().map_or(Ok(()), |why| Err(ProofHalt::Stopped(why)))
    })
    .map_err(|halt| match halt {
        ProofHalt::Stopped(why) => why,
        ProofHalt::Unreadable => Untitled::Unreadable,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Proof {
    /// Prove the checkpoint's recorded generation before streaming.
    Checkpoint,
    /// No per-file generation is recorded (reader hosts record a scan
    /// instant); identity is proven by the source's own header instead.
    None,
}

/// The one recorded source a title may be read from: contained, observed as a
/// regular file, and the only distinct file among the session's locators.
fn select<'a, I>(
    locators: I,
    native: &str,
    roots: &[PathBuf],
    contained: impl Fn(&Path, &[PathBuf]) -> bool,
) -> Result<Candidate<'a>, Untitled>
where
    I: Iterator<Item = (&'a str, &'a IndexedSource)>,
{
    let (mut found, _) = gather(locators, native, roots, contained)?;
    match found.len() {
        1 => Ok(found.pop().expect("one candidate")),
        _ => Err(Untitled::Ambiguous),
    }
}

/// Every distinct recorded source a title may be read from: contained and
/// observed as a regular file. At least one, or why there is none; a recorded
/// source that cannot be looked at refuses them all, because it could be
/// another file claiming this session. Also whether any recorded source was
/// outside its root or an alias, and so was not read.
pub(super) fn gather<'a, I>(
    locators: I,
    native: &str,
    roots: &[PathBuf],
    contained: impl Fn(&Path, &[PathBuf]) -> bool,
) -> Result<(Vec<Candidate<'a>>, bool), Untitled>
where
    I: Iterator<Item = (&'a str, &'a IndexedSource)>,
{
    if identifier_error(native).is_some() {
        return Err(Untitled::InvalidIdentifier);
    }
    // Where the root is itself an alias (a supported layout for the readers),
    // the place it leads is a root too, anchored now.
    let mut anchored: Vec<PathBuf> = Vec::new();
    for root in roots {
        if let Ok(resolved) = root.canonicalize()
            && &resolved != root
        {
            anchored.push(resolved);
        }
        anchored.push(root.clone());
    }
    let mut recorded = 0_usize;
    let mut outside = false;
    let mut unreadable = false;
    let mut found: Vec<Candidate<'a>> = Vec::new();
    for (path, source) in locators {
        recorded += 1;
        let path = Path::new(path);
        if !contained(path, &anchored) {
            // A recorded file that is gone — its directory with it — is
            // missing, not outside: nothing is there to read.
            if !fs::symlink_metadata(path)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                outside = true;
            }
            continue;
        }
        match fs::symlink_metadata(path) {
            Ok(observed) if observed.is_file() => {
                // Two recorded spellings of one file are one candidate.
                let identity = FileIdentity::of(&observed);
                if !found.iter().any(|seen| {
                    let seen = FileIdentity::of(&seen.observed);
                    seen.known
                        && identity.known
                        && seen.dev == identity.dev
                        && seen.ino == identity.ino
                }) {
                    found.push(Candidate {
                        path: path.to_path_buf(),
                        observed,
                        checkpoint: source.checkpoint.as_ref(),
                    });
                }
            }
            // An alias, a directory or a device at the recorded name.
            Ok(_) => outside = true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => unreadable = true,
        }
    }
    match found.len() {
        0 if recorded == 0 => Err(Untitled::NotIndexed),
        0 if unreadable => Err(Untitled::Unreadable),
        0 if outside => Err(Untitled::Outside),
        0 => Err(Untitled::Missing),
        // A recorded source that cannot be looked at could be another file
        // claiming this session, so the ones that can are not known to be all.
        _ if unreadable => Err(Untitled::Unreadable),
        _ => Ok((found, outside)),
    }
}

/// An absolute path with no `.` or `..` step.
fn plain(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|part| !matches!(part, Component::CurDir | Component::ParentDir))
}

/// Whether `directory` exists as a real directory, not an alias to one.
fn real_directory(directory: &Path) -> bool {
    fs::symlink_metadata(directory)
        .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
}

/// `<projects>/<project>/<native>.jsonl`, with a real project directory, as
/// the importer finds a session's own transcript.
pub(super) fn claude_contained(path: &Path, roots: &[PathBuf], native: &str) -> bool {
    let Some(project) = path.parent() else {
        return false;
    };
    plain(path)
        && primary_name(path, native)
        && project
            .parent()
            .is_some_and(|root| roots.iter().any(|allowed| allowed == root))
        && real_directory(project)
}

/// A rollout named for this session below an anchored Codex root, reached
/// through real directories only.
pub(super) fn codex_contained(path: &Path, roots: &[PathBuf], native: &str) -> bool {
    if !plain(path) || codex_segment(path, native).is_none() || !under_roots(path, roots) {
        return false;
    }
    let Some(root) = roots.iter().find(|root| path.starts_with(root)) else {
        return false;
    };
    let mut directory = path.parent();
    while let Some(current) = directory {
        if current == root.as_path() {
            return true;
        }
        if !real_directory(current) {
            return false;
        }
        directory = current.parent();
    }
    false
}

/// Open a file without following an alias, confirm it is still the file that
/// was observed, and capture its identity from the open descriptor.
pub(super) fn open_regular(
    path: &Path,
    observed: &fs::Metadata,
) -> std::io::Result<(fs::File, FileIdentity)> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let source = options.open(path)?;
    let opened = source.metadata()?;
    if !opened.is_file() {
        return Err(std::io::Error::other("source is not a regular file"));
    }
    let (now, then) = (FileIdentity::of(&opened), FileIdentity::of(observed));
    if now.known && then.known && (now.dev != then.dev || now.ino != then.ino) {
        return Err(std::io::Error::other(
            "source changed since it was observed",
        ));
    }
    Ok((source, now))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Flow {
    Continue,
    Stop,
}

/// Read exactly `len` bytes from `source` in bounded chunks, handing every
/// complete line of at most `max_line` bytes to `on_line` without its newline.
/// A longer complete line ends the read: it is never partly parsed, and what
/// it might say is not known. A final line without a newline is not a line (a
/// torn write is left as the importer leaves it), however long. Returns the
/// bytes read, also on failure.
fn scan_lines(
    source: &mut fs::File,
    len: u64,
    max_line: usize,
    chunk: usize,
    stop: &Stop<'_>,
    on_line: &mut dyn FnMut(&[u8]) -> Flow,
) -> Result<u64, (Untitled, u64)> {
    let mut buffer = vec![0_u8; chunk];
    // The start of a line the previous chunk ended inside.
    let mut carry: Vec<u8> = Vec::new();
    let mut overlong = false;
    let mut read_total = 0_u64;
    while read_total < len {
        if let Some(why) = stop.stopped() {
            return Err((why, read_total));
        }
        let want = usize::try_from((len - read_total).min(chunk as u64)).unwrap_or(chunk);
        let read = match source.read(&mut buffer[..want]) {
            // Shorter than the length that was observed: it shrank.
            Ok(0) => return Err((Untitled::Replaced, read_total)),
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err((Untitled::Unreadable, read_total)),
        };
        read_total += read as u64;
        let mut rest = &buffer[..read];
        while let Some(at) = rest.iter().position(|byte| *byte == b'\n') {
            let piece = &rest[..at];
            rest = &rest[at + 1..];
            if overlong || carry.len() + piece.len() > max_line {
                return Err((Untitled::LineTooLong, read_total));
            }
            let flow = if carry.is_empty() {
                on_line(piece)
            } else {
                carry.extend_from_slice(piece);
                on_line(&carry)
            };
            carry.clear();
            if flow == Flow::Stop {
                return Ok(read_total);
            }
        }
        if !overlong {
            if carry.len() + rest.len() <= max_line {
                carry.extend_from_slice(rest);
            } else {
                carry.clear();
                overlong = true;
            }
        }
    }
    Ok(read_total)
}

pub(super) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    let Some((&first, tail)) = needle.split_first() else {
        return true;
    };
    let mut from = 0;
    while let Some(at) = haystack[from..].iter().position(|byte| *byte == first) {
        let start = from + at;
        if haystack[start + 1..].starts_with(tail) {
            return true;
        }
        from = start + 1;
    }
    false
}

pub(super) fn object(line: &[u8]) -> Option<Map<String, Value>> {
    match serde_json::from_slice(line) {
        Ok(Value::Object(object)) => Some(object),
        _ => None,
    }
}

/// A host title as a list may show it: its first line, trimmed, with control
/// and bidirectional-override characters removed, at most
/// [`MAX_TITLE_CHARS`] characters. Anything that is not a nonblank string is
/// not a title.
pub fn host_title(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    let first = text
        .split(['\n', '\r'])
        .find(|line| !line.trim().is_empty())?;
    let cleaned: String = first
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
            {
                ' '
            } else {
                character
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(
        trimmed
            .chars()
            .take(MAX_TITLE_CHARS)
            .collect::<String>()
            .trim_end()
            .to_owned(),
    )
}

/// The two Claude title kinds, each keeping its last valid value.
struct ClaudeTitles<'a> {
    native: &'a str,
    rename: Option<String>,
    generated: Option<String>,
}

impl<'a> ClaudeTitles<'a> {
    fn new(native: &'a str) -> Self {
        Self {
            native,
            rename: None,
            generated: None,
        }
    }

    fn line(&mut self, line: &[u8]) -> Flow {
        // Only a line naming one of the two kinds is parsed at all.
        if !contains(line, b"-title\"") {
            return Flow::Continue;
        }
        let Some(record) = object(line) else {
            return Flow::Continue;
        };
        let (slot, field) = match record.get("type").and_then(Value::as_str) {
            Some("custom-title") => (&mut self.rename, "customTitle"),
            Some("ai-title") => (&mut self.generated, "aiTitle"),
            _ => return Flow::Continue,
        };
        if record.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            return Flow::Continue;
        }
        // A record that names a session names this one, or it is not this
        // session's title.
        if let Some(session) = record.get("sessionId")
            && session.as_str() != Some(self.native)
        {
            return Flow::Continue;
        }
        if let Some(title) = record.get(field).and_then(host_title) {
            *slot = Some(title);
        }
        Flow::Continue
    }

    fn outcome(self) -> TitleOutcome {
        match (self.rename, self.generated) {
            (Some(title), _) => TitleOutcome::Titled {
                title,
                source: TitleSource::ClaudeRename,
            },
            (None, Some(title)) => TitleOutcome::Titled {
                title,
                source: TitleSource::ClaudeGenerated,
            },
            (None, None) => TitleOutcome::Untitled(Untitled::NoTitle),
        }
    }
}

/// What a rollout's opening line says about the history it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Header {
    /// This thread's `session_meta`, for a history held in one rollout.
    Flat,
    /// This thread's `session_meta`, for a paginated or inherited history;
    /// `original` when it continues from no other rollout.
    Paged { original: bool },
    /// Not a `session_meta` at all. A flat rollout may carry its header a few
    /// lines in; a paginated one never does.
    NotFirst,
}

/// Read a rollout's opening line as its header for the thread `native`.
///
/// A paginated or inherited header must name this thread in `payload.id`,
/// and a `history_base` must be an object whose `thread_id` names a rollout.
/// That rollout is where the history continues from; it is not this thread
/// and never names it. `payload.session_id`, when present, names this thread
/// too — except in a spawned thread's header, where Codex writes the parent:
/// there it must be present and name exactly the distinct full parent the
/// typed `thread_spawn` names. A `guardian`, role, path or `history_base`
/// never makes another thread's `session_id` acceptable.
fn codex_header(line: &[u8], native: &str) -> Result<Header, Untitled> {
    if !contains(line, b"session_meta") {
        return Ok(Header::NotFirst);
    }
    let Some(record) = object(line) else {
        return Ok(Header::NotFirst);
    };
    if record.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Ok(Header::NotFirst);
    }
    let payload = record
        .get("payload")
        .and_then(Value::as_object)
        .ok_or(Untitled::IdentityMismatch)?;
    if payload.get("id").and_then(Value::as_str) != Some(native) {
        return Err(Untitled::IdentityMismatch);
    }
    let paginated = payload.get("history_mode").and_then(Value::as_str) == Some("paginated");
    let base = payload.get("history_base").filter(|base| !base.is_null());
    if !paginated && base.is_none() {
        return Ok(Header::Flat);
    }
    let session = match typed_spawn(payload, native) {
        TypedSpawn::None => payload
            .get("session_id")
            .is_none_or(|session| session.as_str() == Some(native)),
        TypedSpawn::Parent(parent) => corroborated(payload, parent),
        TypedSpawn::Malformed | TypedSpawn::SelfSpawn => false,
    };
    if !session {
        return Err(Untitled::IdentityMismatch);
    }
    match base {
        None => Ok(Header::Paged { original: true }),
        Some(base)
            if base
                .get("thread_id")
                .and_then(Value::as_str)
                .is_some_and(uuid_shaped) =>
        {
            Ok(Header::Paged { original: false })
        }
        Some(_) => Err(Untitled::IdentityMismatch),
    }
}

/// A rollout proves it is this session with its own `session_meta`, then its
/// last valid `thread_name_updated` names it.
struct CodexRollout<'a> {
    native: &'a str,
    lines: u64,
    verified: bool,
    refused: Option<Untitled>,
    name: Option<String>,
}

impl<'a> CodexRollout<'a> {
    fn new(native: &'a str) -> Self {
        Self {
            native,
            lines: 0,
            verified: false,
            refused: None,
            name: None,
        }
    }

    fn line(&mut self, line: &[u8]) -> Flow {
        if !self.verified {
            self.lines += 1;
            if contains(line, b"session_meta")
                && let Some(record) = object(line)
                && record.get("type").and_then(Value::as_str) == Some("session_meta")
            {
                let payload = record.get("payload").and_then(Value::as_object);
                let refused = match payload {
                    Some(payload)
                        if payload.get("id").and_then(Value::as_str) == Some(self.native) =>
                    {
                        let paginated = payload.get("history_mode").and_then(Value::as_str)
                            == Some("paginated");
                        let inherited = payload
                            .get("history_base")
                            .is_some_and(|base| !base.is_null());
                        (paginated || inherited).then_some(Untitled::UnsupportedHistory)
                    }
                    _ => Some(Untitled::IdentityMismatch),
                };
                if refused.is_some() {
                    self.refused = refused;
                    return Flow::Stop;
                }
                self.verified = true;
                return Flow::Continue;
            }
            if self.lines >= HEADER_LINES {
                self.refused = Some(Untitled::IdentityMismatch);
                return Flow::Stop;
            }
            return Flow::Continue;
        }
        if !contains(line, b"thread_name_updated") {
            return Flow::Continue;
        }
        let Some(record) = object(line) else {
            return Flow::Continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("event_msg") {
            return Flow::Continue;
        }
        let Some(payload) = record.get("payload").and_then(Value::as_object) else {
            return Flow::Continue;
        };
        if payload.get("type").and_then(Value::as_str) != Some("thread_name_updated") {
            return Flow::Continue;
        }
        if let Some(thread) = payload.get("thread_id")
            && thread.as_str() != Some(self.native)
        {
            return Flow::Continue;
        }
        if let Some(name) = payload.get("thread_name").and_then(host_title) {
            self.name = Some(name);
        }
        Flow::Continue
    }

    fn outcome(self) -> Result<Option<String>, Untitled> {
        if let Some(why) = self.refused {
            return Err(why);
        }
        if !self.verified {
            return Err(Untitled::IdentityMismatch);
        }
        Ok(self.name)
    }
}

/// One sidecar row naming a wanted session with a valid thread name.
fn sidecar_row(line: &[u8], wanted: &[&str]) -> Option<(String, String)> {
    if !contains(line, b"thread_name") {
        return None;
    }
    let row = object(line)?;
    let id = row.get("id")?.as_str()?;
    if !wanted.contains(&id) {
        return None;
    }
    Some((id.to_owned(), host_title(row.get("thread_name")?)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_host_title_is_one_clean_bounded_line() {
        assert_eq!(
            host_title(&json!("  Fix the build  ")).as_deref(),
            Some("Fix the build")
        );
        assert_eq!(
            host_title(&json!("\n\nSecond\nThird")).as_deref(),
            Some("Second")
        );
        assert_eq!(
            host_title(&json!("a\u{202E}b\u{0007}c")).as_deref(),
            Some("a b c")
        );
        let long = "x".repeat(MAX_TITLE_CHARS + 50);
        assert_eq!(
            host_title(&json!(long)).unwrap().chars().count(),
            MAX_TITLE_CHARS
        );
        for invalid in [
            json!(""),
            json!("   \n\t"),
            json!(null),
            json!(7),
            json!(["t"]),
            json!({"t": 1}),
        ] {
            assert_eq!(host_title(&invalid), None, "{invalid}");
        }
    }

    /// A cancel or the deadline that lands while a grown transcript's prefix
    /// is being proven ends the proof between its chunks, as that row's
    /// outcome, rather than after the whole prefix was read.
    #[cfg(unix)]
    #[test]
    fn a_cancel_or_the_deadline_is_heard_during_a_prefix_proof() {
        use sha2::{Digest, Sha256};
        use std::io::Seek;
        const CHUNK: u64 = 64 * 1024;
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("s.jsonl");
        let prefix = vec![b'x'; 4 * CHUNK as usize];
        fs::write(&path, &prefix).unwrap();
        let id = FileIdentity::of(&fs::metadata(&path).unwrap());
        let recorded = checkpoint::file_checkpoint(
            &checkpoint::file_key(&path),
            &id,
            prefix.len() as u64,
            &Sha256::new_with_prefix(&prefix),
            &checkpoint::TailWindow::seeded(prefix.clone()),
            1,
            0,
        );
        let mut grown = prefix.clone();
        grown.extend_from_slice(b"appended\n");
        fs::write(&path, &grown).unwrap();
        let grown = FileIdentity::of(&fs::metadata(&path).unwrap());
        let far = Instant::now() + Duration::from_secs(3600);

        // Proven whole when nothing stops it.
        let mut source = fs::File::open(&path).unwrap();
        let live = Stop {
            cancel: None,
            deadline: far,
        };
        let proven = prove(Some(&recorded), &mut source, &grown, &mut || live.stopped());
        assert_eq!(
            proven.map(|resume| resume.basis).ok(),
            Some(ResumeBasis::Appended)
        );

        // The caller cancels once the first chunk was read.
        let token = CancelToken::new();
        let stop = Stop {
            cancel: Some(&token),
            deadline: far,
        };
        let mut asked = 0;
        let mut source = fs::File::open(&path).unwrap();
        let cancelled = prove(Some(&recorded), &mut source, &grown, &mut || {
            asked += 1;
            if asked == 2 {
                token.cancel();
            }
            stop.stopped()
        });
        assert_eq!(cancelled.err(), Some(Untitled::Cancelled));
        assert_eq!(source.stream_position().unwrap(), CHUNK);

        // The deadline passes once two chunks were read.
        let late = Stop {
            cancel: None,
            deadline: Instant::now(),
        };
        let mut asked = 0;
        let mut source = fs::File::open(&path).unwrap();
        let expired = prove(Some(&recorded), &mut source, &grown, &mut || {
            asked += 1;
            if asked < 4 {
                live.stopped()
            } else {
                late.stopped()
            }
        });
        assert_eq!(expired.err(), Some(Untitled::Deadline));
        assert_eq!(source.stream_position().unwrap(), 2 * CHUNK);
    }

    const THREAD: &str = "019a0000-0000-7000-8000-00000000c0de";

    fn paged_header(thread: &str) -> String {
        format!(
            "{}\n",
            json!({"type": "session_meta", "payload": {"id": thread,
                   "history_mode": "paginated", "history_base": null}})
        )
    }

    fn batch(stop: Stop<'_>) -> Batch<'_> {
        Batch {
            stop,
            limits: TitleLimits::default(),
            charged: 0,
            bytes_read: 0,
        }
    }

    fn observed(path: &Path) -> Candidate<'static> {
        Candidate {
            path: path.to_path_buf(),
            observed: fs::symlink_metadata(path).unwrap(),
            checkpoint: None,
        }
    }

    /// A header is proven against the file as it was observed: an append or a
    /// replacement between observing it and reading its opening line refuses
    /// it, though the line itself still names the thread.
    #[cfg(unix)]
    #[test]
    fn a_rollout_that_changed_since_it_was_observed_proves_no_header() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp
            .path()
            .join(format!("rollout-2026-09-07T12-00-00-{THREAD}.jsonl"));
        let live = || Stop {
            cancel: None,
            deadline: Instant::now() + Duration::from_secs(3600),
        };
        fs::write(&path, paged_header(THREAD)).unwrap();
        let candidate = observed(&path);
        assert_eq!(
            batch(live()).header(&candidate, THREAD),
            Ok(Header::Paged { original: true })
        );

        let mut appended = paged_header(THREAD);
        appended.push_str("{\"type\":\"event_msg\"}\n");
        fs::write(&path, &appended).unwrap();
        assert_eq!(
            batch(live()).header(&candidate, THREAD),
            Err(Untitled::Replaced)
        );

        let candidate = observed(&path);
        let staged = temp.path().join("staged");
        fs::write(&staged, &appended).unwrap();
        fs::rename(&staged, &path).unwrap();
        assert_eq!(
            batch(live()).header(&candidate, THREAD),
            Err(Untitled::Replaced)
        );
    }

    /// A cancel that lands between two headers ends the second before it is
    /// read, and one that lands before the sidecar is read names nothing.
    #[cfg(unix)]
    #[test]
    fn a_cancel_is_heard_between_headers_and_before_the_sidecar() {
        let temp = tempfile::TempDir::new().unwrap();
        let first = temp.path().join("first.jsonl");
        let second = temp.path().join("second.jsonl");
        fs::write(&first, paged_header(THREAD)).unwrap();
        fs::write(&second, paged_header(THREAD)).unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(
            home.join(".codex/session_index.jsonl"),
            format!("{}\n", json!({"id": THREAD, "thread_name": "Named"})),
        )
        .unwrap();
        let token = CancelToken::new();
        let mut reading = batch(Stop {
            cancel: Some(&token),
            deadline: Instant::now() + Duration::from_secs(3600),
        });
        assert_eq!(
            reading.header(&observed(&first), THREAD),
            Ok(Header::Paged { original: true })
        );
        assert_eq!(reading.sidecar(&home, &[THREAD]).len(), 1);
        let read_so_far = reading.bytes_read;
        token.cancel();
        assert_eq!(
            reading.header(&observed(&second), THREAD),
            Err(Untitled::Cancelled)
        );
        assert!(reading.sidecar(&home, &[THREAD]).is_empty());
        assert_eq!(reading.bytes_read, read_so_far, "nothing more was read");
    }

    #[test]
    fn a_continuation_name_holds_the_thread_then_a_uuid_rollout() {
        let rollout = "019a0000-0000-7000-8000-0000000a0001";
        let name = |file: &str| codex_segment(Path::new(file), THREAD);
        assert_eq!(
            name(&format!("rollout-2026-09-07T12-00-00-{THREAD}.jsonl")),
            Some(Segment::Root)
        );
        assert_eq!(
            name(&format!(
                "rollout-2026-09-07T12-00-00-{THREAD}_{rollout}.jsonl"
            )),
            Some(Segment::Continuation(rollout.to_owned()))
        );
        for miss in [
            format!("rollout-2026-09-07T12-00-00-{rollout}_{THREAD}.jsonl"),
            format!("rollout-{THREAD}_{rollout}.jsonl"),
            format!("rollout-2026-09-07T12-00-00-{THREAD}_{rollout}x.jsonl"),
            format!(
                "rollout-2026-09-07T12-00-00-{THREAD}_{}.jsonl",
                &rollout[..35]
            ),
            format!("rollout-2026-09-07T12-00-00-{THREAD}_{rollout}.json"),
            format!("session-2026-09-07T12-00-00-{THREAD}_{rollout}.jsonl"),
            format!("rollout-2026-09-07T12-00-00-x{THREAD}_{rollout}.jsonl"),
        ] {
            assert_eq!(name(&miss), None, "{miss}");
        }
        // A thread that is not UUID-shaped has no continuations.
        assert_eq!(
            codex_segment(
                Path::new(&format!(
                    "rollout-2026-09-07T12-00-00-thread_{rollout}.jsonl"
                )),
                "thread"
            ),
            None
        );
    }

    #[test]
    fn a_needle_is_found_anywhere_and_only_whole() {
        assert!(contains(br#"{"type":"ai-title"}"#, b"-title\""));
        assert!(contains(b"--title\"", b"-title\""));
        assert!(!contains(b"-titl", b"-title\""));
        assert!(!contains(b"", b"x"));
    }
}
