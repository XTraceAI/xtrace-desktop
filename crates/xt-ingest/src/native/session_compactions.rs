//! Recorded compactions from requested indexed sources and explicit history dependencies.
//! Counts and revision facts live only in bounded native memory; no text or paths in IPC.
//! A forked Codex conversation's own count is kept apart from the count it
//! inherits: the compactions of the conversation it was forked from, recorded
//! before the fork point. An older fork whose file holds a copy of that
//! history, with no reference to say where it ends, is split by compaction
//! identity: the copied ones are those the other conversation also recorded.
use super::checkpoint::{self, FileIdentity, ResumeBasis};
use super::claude_launch::group::Role;
use super::readers_cli::CancelToken;
use super::session_source::{IndexedSource, unchanged};
use super::session_titles::{
    Segment, TitleTarget, Untitled, claude_contained, codex_contained, codex_segment,
    full_thread_id, gather, open_regular, title_sources, uuid_shaped,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use xt_store::{
    Host, SessionSource, Store,
    batch::{LocatorRows, escape_like},
};
mod jsonl;

pub const MAX_SESSIONS: usize = 50;
const VERSION: u32 = 4;
const MAX_CACHE: usize = 256;
const MAX_EVENTS: usize = 200_000;
const MAX_SNAPSHOT: u64 = 1024 * 1024 * 1024;
const MAX_BATCH: u64 = 2 * 1024 * 1024 * 1024;
const MAX_DECODED: usize = 128 * 1024 * 1024;
const MAX_NODES: usize = 200_000;
const MAX_DEPTH: usize = 128;
const MAX_SQL_BYTES: i64 = 1024 * 1024;
/// The most referenced rollouts a fork's inherited history may chain through.
const MAX_REFERENCES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    NotIndexed,
    Unsupported,
    Missing,
    Unreadable,
    Ambiguous,
    Replaced,
    IdentityMismatch,
    Ownership,
    Incomplete,
    Limit,
    Cancelled,
    /// An approval reviewer's file holds only part of the reviewed
    /// conversation's history, whose compactions it carries, and none of its
    /// own: earlier compactions were left out.
    Snapshot,
    /// A forked conversation's file holds history copied from the conversation
    /// it was forked from, and its own compactions could not be told apart
    /// from the copied ones.
    CopiedHistory,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    Count { count: u32 },
    Unknown { reason: Reason },
}
impl Outcome {
    pub fn unknown(reason: Reason) -> Self {
        Self::Unknown { reason }
    }
}
/// How a recorded compaction started, as its own marker says. Codex markers
/// record no trigger, so they are always `Unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Auto,
    Manual,
    Unknown,
}
/// When one counted compaction was recorded (UTC milliseconds) and its trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Event {
    pub at_ms: i64,
    pub trigger: Trigger,
}
/// A count with the recorded time of each counted compaction that has one,
/// oldest first. A counted marker with a missing or unreadable time keeps
/// its place in the count and simply has no event, so `events.len()` is
/// at most the count; Cursor summaries carry no time and have no events.
/// `events` is empty whenever the outcome is unknown.
///
/// `inherited` is set only for a forked Codex conversation whose own count is
/// known: the compactions recorded in the conversation it was forked from
/// (following its own references) before the fork point, or why that could
/// not be counted; for an older fork whose file holds a copy of that history,
/// the copied compactions, and `copied` is set. `outcome` and `events` stay
/// the fork's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counted {
    pub outcome: Outcome,
    pub events: Vec<Event>,
    pub inherited: Option<Outcome>,
    /// `inherited` counts history copied into this conversation's own file,
    /// not history it references.
    pub copied: bool,
}
impl Counted {
    pub fn unknown(reason: Reason) -> Self {
        Self {
            outcome: Outcome::unknown(reason),
            events: Vec::new(),
            inherited: None,
            copied: false,
        }
    }
}
impl From<Untitled> for Reason {
    fn from(value: Untitled) -> Self {
        match value {
            Untitled::Missing => Self::Missing,
            Untitled::NotIndexed => Self::NotIndexed,
            Untitled::Outside | Untitled::InvalidIdentifier | Untitled::IdentityMismatch => {
                Self::IdentityMismatch
            }
            Untitled::Ambiguous => Self::Ambiguous,
            Untitled::Replaced => Self::Replaced,
            Untitled::Unreadable => Self::Unreadable,
            Untitled::Cancelled => Self::Cancelled,
            Untitled::UnsupportedHistory | Untitled::UnsupportedHost => Self::Unsupported,
            _ => Self::Limit,
        }
    }
}
pub type Target = TitleTarget;
pub fn sources(store: &Store, host: Host, native: &str) -> xt_store::Result<Vec<IndexedSource>> {
    if host != Host::Cursor {
        return title_sources(store, host, native);
    }
    if !uuid_shaped(native) {
        return Ok(Vec::new());
    }
    let pattern = format!("cursor:%/{}/store.db", escape_like(native));
    let LocatorRows::Complete(rows) =
        store.source_cursors_like_bounded(SessionSource::ReadersCli, &[&pattern])?
    else {
        return Ok(Vec::new());
    };
    Ok(rows
        .into_iter()
        .map(|row| IndexedSource {
            locator: row.cursor_key,
            checkpoint: None,
        })
        .collect())
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Revision(Vec<(PathBuf, Option<FileIdentity>)>);
#[derive(Clone, Debug, Default)]
struct Facts {
    /// Counted identity → its recorded time, when it has one.
    events: BTreeMap<String, Option<Event>>,
    uncertain: bool,
}
impl Facts {
    /// Count an identity once; a repeat only fills a time the first lacked.
    fn count(&mut self, id: String, event: Option<Event>) {
        let slot = self.events.entry(id).or_insert(None);
        if slot.is_none() {
            *slot = event;
        }
    }
}
#[derive(Clone)]
struct Entry {
    revisions: Vec<Revision>,
    /// The referenced rollouts a fork's inherited count was read from, by
    /// rollout identity: still the one file of that name, unchanged.
    references: Vec<(String, Revision)>,
    /// A copied-history fork's origin and the files named for it that its
    /// split was read from: still exactly those files, each unchanged.
    origin: Option<(String, Vec<Revision>)>,
    counted: Counted,
}
#[derive(Default)]
pub struct Reader {
    cache: BTreeMap<String, Entry>,
    pub scans: u64,
}
struct Work<'a> {
    deadline: Instant,
    cancel: &'a CancelToken,
    bytes: u64,
    decoded: usize,
    nodes: usize,
}
impl Work<'_> {
    fn check(&self) -> Result<(), Reason> {
        if self.cancel.is_cancelled() {
            Err(Reason::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(Reason::Limit)
        } else {
            Ok(())
        }
    }
    fn charge(&mut self, bytes: u64) -> Result<(), Reason> {
        self.check()?;
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > MAX_BATCH {
            Err(Reason::Limit)
        } else {
            Ok(())
        }
    }
}
struct Snapshot {
    db: Connection,
    _dir: tempfile::TempDir,
    revision: Revision,
}
impl Reader {
    pub fn read(&mut self, home: &Path, targets: &[Target], cancel: &CancelToken) -> Vec<Outcome> {
        self.read_events(home, targets, cancel)
            .into_iter()
            .map(|counted| counted.outcome)
            .collect()
    }
    /// The same read as [`Reader::read`], keeping each counted compaction's
    /// recorded time and trigger. Counts and their cache are shared.
    pub fn read_events(
        &mut self,
        home: &Path,
        targets: &[Target],
        cancel: &CancelToken,
    ) -> Vec<Counted> {
        let mut work = Work {
            deadline: Instant::now() + Duration::from_secs(15),
            cancel,
            bytes: 0,
            decoded: 0,
            nodes: 0,
        };
        if targets.len() > MAX_SESSIONS {
            return vec![Counted::unknown(Reason::Limit); targets.len()];
        }
        let mut snapshots = BTreeMap::<PathBuf, Snapshot>::new();
        let mut result = Vec::new();
        for target in targets {
            let answer = (|| {
                work.check()?;
                let files = target_files(home, target, &work)?;
                let revisions = files
                    .iter()
                    .map(|path| revision(path))
                    .collect::<Result<Vec<_>, _>>()?;
                if revisions.iter().any(|r| r.0[0].1.is_none()) {
                    return Err(Reason::Missing);
                }
                let key = format!("{home:?}/{VERSION}/{target:?}");
                if let Some(entry) = self.cache.get(&key).filter(|e| {
                    e.revisions == revisions
                        && e.references.iter().all(|(rollout, r)| {
                            referenced(home, rollout, &work).is_ok_and(|(path, _)| path == r.0[0].0)
                                && revision(&r.0[0].0).as_ref() == Ok(r)
                        })
                        && e.origin.as_ref().is_none_or(|(origin, read)| {
                            codex_files(home, origin, &work).is_ok_and(|now| {
                                now.len() == read.len()
                                    && now
                                        .iter()
                                        .zip(read)
                                        .all(|(path, r)| revision(path).as_ref() == Ok(r))
                            })
                        })
                }) {
                    return Ok(entry.counted.clone());
                }
                self.scans += 1;
                let (mut facts, lineage) = if target.host == Host::Cursor {
                    let path = &files[0];
                    if !snapshots.contains_key(path) {
                        let dir = snapshot(path, &revisions[0], &mut work)?;
                        let db = Connection::open(
                            dir.path()
                                .join(path.file_name().ok_or(Reason::IdentityMismatch)?),
                        )
                        .map_err(|_| Reason::Unreadable)?;
                        db.busy_timeout(Duration::from_millis(50))
                            .map_err(|_| Reason::Unreadable)?;
                        // Recover a hot journal in the private copy before
                        // query_only. The original is never opened by SQLite.
                        db.query_row("PRAGMA schema_version", [], |_| Ok(()))
                            .map_err(|_| Reason::Unreadable)?;
                        db.execute_batch("PRAGMA query_only=ON; BEGIN;")
                            .map_err(|_| Reason::Unreadable)?;
                        snapshots.insert(
                            path.clone(),
                            Snapshot {
                                db,
                                _dir: dir,
                                revision: revisions[0].clone(),
                            },
                        );
                    }
                    let snapshot = snapshots.get(path).expect("snapshot inserted");
                    if snapshot.revision != revisions[0] {
                        return Err(Reason::Replaced);
                    }
                    let facts = cursor_target(
                        &snapshot.db,
                        snapshot._dir.path(),
                        path,
                        &target.native_session_id,
                        &mut work,
                    )?;
                    (facts, Lineage::Own)
                } else {
                    jsonl_target(home, target, &files, &mut work)?
                };
                work.check()?;
                if target_files(home, target, &work)? != files
                    || files
                        .iter()
                        .map(|path| revision(path))
                        .collect::<Result<Vec<_>, _>>()?
                        != revisions
                {
                    return Err(Reason::Replaced);
                }
                // A copied-history fork's own count is what is left once the
                // copied compactions are told apart; with none at all, every
                // row's count is its own: zero.
                let mut origin = None;
                let mut copied = None;
                if let Lineage::Copied {
                    origin: from,
                    order,
                } = &lineage
                    && !order.is_empty()
                {
                    let (from, read, ids) = copied_split(
                        home,
                        from.as_deref(),
                        &target.native_session_id,
                        order,
                        &mut work,
                    )?;
                    for id in &order[..ids] {
                        facts.events.remove(id);
                    }
                    copied = Some(ids);
                    origin = Some((from, read));
                }
                let mut answer = facts_counted(&facts)?;
                if let Some(ids) = copied {
                    answer.inherited = Some(Outcome::Count {
                        count: u32::try_from(ids).map_err(|_| Reason::Limit)?,
                    });
                    answer.copied = true;
                }
                // The own count is proven: a fork's inherited count, read
                // after it, can only leave its own part unknown.
                let mut references = Vec::new();
                if let Lineage::Fork(fork) = &lineage {
                    answer.inherited = Some(
                        match inherited(home, fork, &facts, &mut references, &mut work) {
                            Ok(count) => Outcome::Count { count },
                            Err(why) => Outcome::unknown(why),
                        },
                    );
                }
                // An unknown inherited count is never kept: what it rests on
                // (a missing, duplicate or changing file) is not all held to
                // be checked again. Its own count is read again with it.
                let settled = !matches!(answer.inherited, Some(Outcome::Unknown { .. }));
                // Eviction is only a future miss; proof uses locally held revisions.
                if settled {
                    if self.cache.len() >= MAX_CACHE {
                        self.cache.clear();
                    }
                    self.cache.insert(
                        key,
                        Entry {
                            revisions,
                            references,
                            origin,
                            counted: answer.clone(),
                        },
                    );
                }
                Ok(answer)
            })()
            .unwrap_or_else(Counted::unknown);
            result.push(answer);
        }
        // Validate shared snapshot source revisions once more before returning.
        for (path, snapshot) in &snapshots {
            if revision(path).as_ref() != Ok(&snapshot.revision) {
                for (target, outcome) in targets.iter().zip(&mut result) {
                    if target.host == Host::Cursor
                        && target_files(home, target, &work).is_ok_and(|files| files.contains(path))
                    {
                        *outcome = Counted::unknown(Reason::Replaced);
                    }
                }
            }
        }
        if cancel.is_cancelled() {
            result.fill(Counted::unknown(Reason::Cancelled));
        }
        result
    }
}
fn facts_counted(facts: &Facts) -> Result<Counted, Reason> {
    if facts.uncertain {
        return Err(Reason::Ownership);
    }
    let count = u32::try_from(facts.events.len()).map_err(|_| Reason::Limit)?;
    let mut events: Vec<Event> = facts.events.values().flatten().copied().collect();
    events.sort();
    Ok(Counted {
        outcome: Outcome::Count { count },
        events,
        inherited: None,
        copied: false,
    })
}
/// A marker's own recorded time, in UTC milliseconds; absent when unreadable.
fn recorded_at(side: &jsonl::Side) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(side.timestamp.get()?)
        .ok()
        .map(|at| at.timestamp_millis())
}
fn target_files(home: &Path, target: &Target, work: &Work<'_>) -> Result<Vec<PathBuf>, Reason> {
    let native = &target.native_session_id;
    if !uuid_shaped(native) {
        return Err(Reason::IdentityMismatch);
    }
    match target.host {
        Host::Claude | Host::Codex => {
            if target.locators.len() > 8 {
                return Err(Reason::Limit);
            }
            let roots = vec![home.join(if target.host == Host::Claude {
                ".claude/projects"
            } else {
                ".codex/sessions"
            })];
            let (files, outside) = gather(
                target.locators.iter().filter_map(|source| {
                    Some((
                        source
                            .locator
                            .strip_prefix(if target.host == Host::Claude {
                                "claude:"
                            } else {
                                "codex:"
                            })?,
                        source,
                    ))
                }),
                native,
                &roots,
                |path, roots| {
                    safe(path, home)
                        && if target.host == Host::Claude {
                            claude_contained(path, roots, native)
                        } else {
                            codex_contained(path, roots, native)
                        }
                },
            )
            .map_err(Reason::from)?;
            if outside || files.len() != target.locators.len() {
                return Err(Reason::Incomplete);
            }
            if target.host == Host::Claude && files.len() != 1 {
                return Err(Reason::Ambiguous);
            }
            if target.host == Host::Codex {
                let discovered = codex_files(home, native, work)?;
                if files.iter().any(|file| !discovered.contains(&file.path)) {
                    return Err(Reason::Incomplete);
                }
                Ok(discovered)
            } else {
                Ok(files.into_iter().map(|file| file.path).collect())
            }
        }
        Host::Cursor => {
            let root = home.join(".cursor/chats");
            let mut stores = BTreeSet::new();
            for source in &target.locators {
                let path = PathBuf::from(
                    source
                        .locator
                        .strip_prefix("cursor:")
                        .ok_or(Reason::IdentityMismatch)?,
                );
                if !safe(&path, home)
                    || !path.starts_with(&root)
                    || path
                        .parent()
                        .and_then(Path::file_name)
                        .and_then(|v| v.to_str())
                        != Some(native)
                    || !path.ends_with("store.db")
                {
                    return Err(Reason::IdentityMismatch);
                }
                stores.insert(path);
            }
            // Names only, never parse unrelated stores. Older indexed Cursor
            // representations can omit a store locator.
            if stores.is_empty() {
                if !safe(&root, home) {
                    return Err(Reason::IdentityMismatch);
                }
                let names = list(&root)?;
                if names.len() > 4096 {
                    return Err(Reason::Limit);
                }
                for project in names {
                    if !safe(&project, home) {
                        // Ignore unrelated aliases without following them.
                        continue;
                    }
                    let path = project.join(native).join("store.db");
                    if fs::symlink_metadata(&path).is_ok() {
                        if !safe(&path, home) {
                            return Err(Reason::IdentityMismatch);
                        }
                        stores.insert(path);
                    }
                }
            }
            match stores.len() {
                1 => Ok(stores.into_iter().collect()),
                0 => {
                    let path = home
                        .join("Library/Application Support/Cursor/User/globalStorage/state.vscdb");
                    if safe(&path, home) {
                        Ok(vec![path])
                    } else {
                        Err(Reason::IdentityMismatch)
                    }
                }
                _ => Err(Reason::Ambiguous),
            }
        }
        Host::Other => Err(Reason::Unsupported),
    }
}
// Filename discovery only: bodies of other threads are never opened. Match
// the supported root and up to YYYY/MM/DD, using real directories only.
fn codex_files(home: &Path, native: &str, work: &Work<'_>) -> Result<Vec<PathBuf>, Reason> {
    codex_names(home, work, |path| codex_segment(path, native).is_some())
}
/// The one rollout named with the immutable rollout identity `rollout`, as
/// the reader's `references` finds it, and the thread its name gives it.
fn referenced(home: &Path, rollout: &str, work: &Work<'_>) -> Result<(PathBuf, String), Reason> {
    let named = |path: &Path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(super::claude_launch::group::census_name)
    };
    let found = codex_names(home, work, |path| {
        named(path).is_some_and(|(_, id, _)| id == rollout)
    })?;
    let [path] = found.as_slice() else {
        return Err(if found.is_empty() {
            Reason::Missing
        } else {
            Reason::Ambiguous
        });
    };
    let (thread, _, _) = named(path).ok_or(Reason::IdentityMismatch)?;
    Ok((path.clone(), thread))
}
fn codex_names(
    home: &Path,
    work: &Work<'_>,
    wanted: impl Fn(&Path) -> bool,
) -> Result<Vec<PathBuf>, Reason> {
    let root = home.join(".codex/sessions");
    if !safe(&root, home) {
        return Err(Reason::IdentityMismatch);
    }
    let mut pending = vec![(root, 0)];
    let mut files = BTreeSet::new();
    let mut entries = 0usize;
    while let Some((directory, depth)) = pending.pop() {
        work.check()?;
        let listing = fs::read_dir(directory).map_err(|_| Reason::Unreadable)?;
        for entry in listing {
            work.check()?;
            entries += 1;
            if entries > 2_000_000 {
                return Err(Reason::Limit);
            }
            let entry = entry.map_err(|_| Reason::Unreadable)?;
            let path = entry.path();
            if wanted(&path) {
                if !safe(&path, home) {
                    return Err(Reason::IdentityMismatch);
                }
                regular(&path)?;
                files.insert(path);
                if files.len() > 16 {
                    return Err(Reason::Limit);
                }
            } else if depth < 3 && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                pending.push((path, depth + 1));
            }
        }
    }
    Ok(files.into_iter().collect())
}
fn ownership_metadata(value: &Value, native: &str) -> bool {
    ["originalSessionId", "sourceSessionId"]
    .iter()
    .any(|key| !value[*key].is_null() && value[*key].as_str() != Some(native))
        // These existing fork/parent fields do not establish ownership or a
        // supported inherited-history split, even when they name this root.
        || ["forkedFromSessionId", "parentSessionId", "parentComposerId"]
            .iter().any(|key| !value[*key].is_null())
        || value["isFork"].as_bool() == Some(true)
}
#[derive(Clone)]
struct Header {
    rollout: String,
    base: Option<Value>,
    boundary: Option<u64>,
    first: Option<u64>,
    paginated: bool,
    /// The different conversation it says it was forked from, when that and
    /// any fork cutoff it states agree with its base.
    fork: Option<String>,
    role: Role,
    /// Its rows may hold history copied from the conversation it names, with
    /// nothing to say where that ends: that name, when it is text.
    copied: Option<Option<String>>,
}
/// Where a thread's history comes from, beyond its own files.
enum Lineage {
    Own,
    /// A fork whose first rollout references the other conversation's.
    Fork(Fork),
    /// An older fork whose file holds a copy of the other conversation's
    /// history: the conversation it names, and its counted compaction
    /// identities in the order they were recorded.
    Copied {
        origin: Option<String>,
        order: Vec<String>,
    },
}
/// A forked conversation's first rollout: the conversation it was forked
/// from, and the rollout, byte cutoff and ordinal cutoff its history
/// continues.
struct Fork {
    parent: String,
    base: (String, u64, u64),
    /// The fork's own rollout identities.
    own: BTreeSet<String>,
}
fn codex_header(row: &Value, path: &Path, native: &str) -> Result<Header, Reason> {
    let checked = super::claude_launch::group::header(
        &serde_json::to_vec(row).map_err(|_| Reason::Incomplete)?,
        native,
    )
    .map_err(|_| Reason::Ownership)?;
    let rollout = match codex_segment(path, native).ok_or(Reason::IdentityMismatch)? {
        Segment::Root => native.to_owned(),
        Segment::Continuation(id) => id,
    };
    if ownership_metadata(&row["payload"], native) {
        return Err(Reason::Ownership);
    }
    let paginated = checked.paginated;
    let base = checked.base.map(|(id, bytes, ordinal)| {
        serde_json::json!({
            "thread_id": id, "end_byte_offset": bytes, "end_ordinal_exclusive": ordinal
        })
    });
    let boundary = checked.inherited_below.map(|n| n as u64);
    let first = checked.ordinal.map(|n| n as u64);
    if rollout != native && (!paginated || base.is_none()) {
        return Err(Reason::Unsupported);
    }
    let fork = checked
        .forked
        .then(|| row["payload"]["forked_from_id"].as_str().map(str::to_owned))
        .flatten();
    let copied = checked
        .copied
        .then(|| row["payload"]["forked_from_id"].as_str().map(str::to_owned));
    let start = base
        .as_ref()
        .map_or(Some(0), |base| base["end_ordinal_exclusive"].as_u64());
    if paginated && first != start {
        return Err(Reason::Incomplete);
    }
    Ok(Header {
        rollout,
        base,
        boundary,
        first,
        paginated,
        fork,
        role: checked.role,
        copied,
    })
}
fn jsonl_target(
    home: &Path,
    target: &Target,
    paths: &[PathBuf],
    work: &mut Work<'_>,
) -> Result<(Facts, Lineage), Reason> {
    let native = &target.native_session_id;
    let roots = vec![home.join(if target.host == Host::Claude {
        ".claude/projects"
    } else {
        ".codex/sessions"
    })];
    let selected: Vec<IndexedSource> = paths
        .iter()
        .map(|path| {
            target
                .locators
                .iter()
                .find(|source| {
                    source
                        .locator
                        .split_once(':')
                        .is_some_and(|(_, p)| Path::new(p) == path)
                })
                .cloned()
                .unwrap_or_else(|| IndexedSource {
                    locator: format!("codex:{}", path.display()),
                    checkpoint: None,
                })
        })
        .collect();
    let (files, outside) = gather(
        selected.iter().filter_map(|source| {
            Some((
                source
                    .locator
                    .strip_prefix(if target.host == Host::Claude {
                        "claude:"
                    } else {
                        "codex:"
                    })?,
                source,
            ))
        }),
        native,
        &roots,
        |path, roots| {
            safe(path, home)
                && if target.host == Host::Claude {
                    claude_contained(path, roots, native)
                } else {
                    codex_contained(path, roots, native)
                }
        },
    )
    .map_err(Reason::from)?;
    if outside {
        return Err(Reason::IdentityMismatch);
    }
    let mut headers = Vec::new();
    let mut lineage = Lineage::Own;
    if target.host == Host::Codex {
        for file in &files {
            let observed = regular(&file.path)?;
            let (mut source, identity) =
                open_regular(&file.path, &observed).map_err(|_| Reason::Replaced)?;
            let row = jsonl::Rows::new(&mut source, identity.len, work).header()?;
            verify_jsonl(&source, &file.path, &identity)?;
            let header = codex_header(&row, &file.path, native)?;
            headers.push(header);
        }
        let by_id: BTreeMap<_, _> = headers.iter().map(|h| (h.rollout.as_str(), h)).collect();
        if by_id.len() != headers.len() {
            return Err(Reason::Ambiguous);
        }
        if headers.iter().any(|h| !h.paginated) && headers.len() > 1 {
            return Err(Reason::Ambiguous);
        }
        // The first rollout is the thread's own original or, as the reader's
        // `codex_history.plan` has it, a fork's whose base is another
        // conversation's rollout outside this group.
        let first = |h: &Header| match &h.base {
            None => true,
            Some(base) => {
                h.fork.is_some()
                    && base["thread_id"]
                        .as_str()
                        .is_some_and(|id| !by_id.contains_key(id))
            }
        };
        let roots: Vec<&Header> = headers.iter().filter(|h| first(h)).collect();
        let [root] = roots.as_slice() else {
            return Err(Reason::Incomplete);
        };
        if root.rollout != *native {
            return Err(Reason::Incomplete);
        }
        // A copied history is split only within its one file.
        if let Some(origin) = &root.copied {
            if headers.len() != 1 {
                return Err(Reason::CopiedHistory);
            }
            lineage = Lineage::Copied {
                origin: origin.clone(),
                order: Vec::new(),
            };
        }
        if let (Some(parent), Some(base)) = (&root.fork, &root.base) {
            lineage = Lineage::Fork(Fork {
                parent: parent.clone(),
                base: (
                    base["thread_id"]
                        .as_str()
                        .ok_or(Reason::Incomplete)?
                        .to_owned(),
                    base["end_byte_offset"].as_u64().ok_or(Reason::Incomplete)?,
                    base["end_ordinal_exclusive"]
                        .as_u64()
                        .ok_or(Reason::Incomplete)?,
                ),
                own: by_id.keys().map(|id| (*id).to_owned()).collect(),
            });
        }
        for header in &headers {
            let mut visited = BTreeSet::new();
            let mut current = header;
            while let Some(base) = current.base.as_ref().filter(|_| !first(current)) {
                let id = base["thread_id"].as_str().ok_or(Reason::Ownership)?;
                if !visited.insert(id) {
                    return Err(Reason::Incomplete);
                }
                if base["end_byte_offset"].as_u64().is_none()
                    || base["end_ordinal_exclusive"].as_u64().is_none()
                {
                    return Err(Reason::Incomplete);
                }
                current = by_id.get(id).copied().ok_or(Reason::Incomplete)?;
            }
        }
    }
    let mut facts = Facts::default();
    let mut claude_owner = false;
    // An inherited compaction row in a file whose marker is not a spawned
    // thread's, and whether that file is an approval reviewer's.
    let mut inherited_marker = false;
    let mut reviewer_snapshot = false;
    for (file_index, file) in files.iter().enumerate() {
        // Refuse a torn final line; title reads intentionally tolerate it.
        let observed = regular(&file.path)?;
        let (mut source, identity) =
            open_regular(&file.path, &observed).map_err(|_| Reason::Replaced)?;
        if identity.len == 0 {
            return Err(Reason::Incomplete);
        }
        source
            .seek(SeekFrom::End(-1))
            .map_err(|_| Reason::Unreadable)?;
        let mut last = [0];
        work.charge(1)?;
        source
            .read_exact(&mut last)
            .map_err(|_| Reason::Unreadable)?;
        if last[0] != b'\n' {
            return Err(Reason::Incomplete);
        }
        if identity.len > MAX_BATCH.saturating_sub(work.bytes) {
            return Err(Reason::Limit);
        }
        if target.host == Host::Claude {
            let checkpoint = selected[file_index].checkpoint.as_ref();
            if checkpoint.is_some() {
                // Reserve the worst-case proof read even if proof fails.
                // A matching checkpoint may hash the entire prefix.
                work.charge(identity.len)?;
            }
            let resume =
                checkpoint::resume_point_checked(checkpoint, &mut source, &identity, &mut || {
                    work.check().map_err(CompactionProofError::Stopped)
                })
                .map_err(|error| match error {
                    CompactionProofError::Stopped(why) => why,
                    CompactionProofError::Io => Reason::Unreadable,
                })?;
            if matches!(
                resume.basis,
                ResumeBasis::Replaced | ResumeBasis::Truncated | ResumeBasis::Rewritten
            ) {
                return Err(Reason::Replaced);
            }
        }
        source
            .seek(SeekFrom::Start(0))
            .map_err(|_| Reason::Unreadable)?;
        let header = headers.get(file_index);
        let cutoffs: Vec<(u64, u64)> = header.map_or(Vec::new(), |h| {
            headers
                .iter()
                .filter_map(|other| {
                    let base = other.base.as_ref()?;
                    (base["thread_id"].as_str() == Some(&h.rollout)).then_some((
                        base["end_byte_offset"].as_u64()?,
                        base["end_ordinal_exclusive"].as_u64()?,
                    ))
                })
                .collect()
        });
        let mut proven = BTreeSet::new();
        let mut lines = 0u64;
        let mut next = header.and_then(|h| h.first);
        let mut rows = jsonl::Rows::new(&mut source, identity.len, work);
        while let Some((row, offset)) = rows.next(target.host == Host::Codex)? {
            // The marker's time and trigger, read beside the projection.
            let side = rows.side();
            lines += 1;
            if lines > MAX_EVENTS as u64 {
                return Err(Reason::Limit);
            }
            if target.host == Host::Claude {
                if matches!(row["type"].as_str(), Some("user" | "assistant")) {
                    if row["sessionId"].as_str() == Some(native) {
                        claude_owner = true;
                    } else {
                        facts.uncertain = true;
                    }
                }
                if ownership_metadata(&row, native) {
                    facts.uncertain = true;
                }
                if row["type"] == "system" && row["subtype"] == "compact_boundary" {
                    let id = row["uuid"]
                        .as_str()
                        .filter(|id| uuid_shaped(id))
                        .ok_or(Reason::Ownership)?;
                    if row["sessionId"].as_str() != Some(native) {
                        facts.uncertain = true;
                    } else if row["isSidechain"].as_bool() == Some(false) {
                        claude_owner = true;
                        let trigger = match side.trigger.get() {
                            Some("auto") => Trigger::Auto,
                            Some("manual") => Trigger::Manual,
                            _ => Trigger::Unknown,
                        };
                        let event = recorded_at(&side).map(|at_ms| Event { at_ms, trigger });
                        facts.count(id.to_owned(), event);
                    } else if row["isSidechain"].as_bool().is_none() {
                        facts.uncertain = true;
                    }
                }
            } else {
                let header = header.ok_or(Reason::Incomplete)?;
                let ordinal = row["ordinal"].as_u64();
                if next.is_some() && ordinal != next {
                    return Err(Reason::Incomplete);
                }
                if let Some(ordinal) = ordinal {
                    next = ordinal.checked_add(1);
                }
                for (bytes, end) in &cutoffs {
                    if offset == *bytes && next == Some(*end) {
                        proven.insert((*bytes, *end));
                    }
                }
                let inherited = header
                    .boundary
                    .zip(ordinal)
                    .is_some_and(|(from, ordinal)| ordinal < from);
                if lines > 1 && row["type"] == "session_meta" && !inherited {
                    return Err(Reason::IdentityMismatch);
                }
                if row["type"] == "compacted" {
                    if inherited {
                        // A spawned thread's own marker says which rows it
                        // was handed: they are never its own, and none of
                        // its own is missing beside them.
                        match header.role {
                            Role::Spawned => {}
                            Role::Reviewer => {
                                inherited_marker = true;
                                reviewer_snapshot = true;
                            }
                            Role::Own => inherited_marker = true,
                        }
                    } else {
                        let payload = &row["payload"];
                        let id = payload["compaction_response_id"]
                            .as_str()
                            .filter(|id| !id.is_empty())
                            .or_else(|| payload["window_id"].as_str().filter(|id| !id.is_empty()))
                            .ok_or(Reason::Ownership)?;
                        if id.len() > 256 {
                            return Err(Reason::Limit);
                        }
                        if ownership_metadata(payload, native) {
                            facts.uncertain = true;
                        }
                        // Codex records no auto/manual marker on a compaction.
                        let event = recorded_at(&side).map(|at_ms| Event {
                            at_ms,
                            trigger: Trigger::Unknown,
                        });
                        if let Lineage::Copied { order, .. } = &mut lineage
                            && !facts.events.contains_key(id)
                        {
                            order.push(id.to_owned());
                        }
                        facts.count(id.to_owned(), event);
                    }
                }
            }
            if facts.events.len() > MAX_EVENTS {
                return Err(Reason::Limit);
            }
        }
        verify_jsonl(&source, &file.path, &identity)?;
        if header.is_some_and(|h| {
            h.boundary
                .is_some_and(|from| next.is_none_or(|end| from > end))
        }) {
            return Err(Reason::Incomplete);
        }
        for (bytes, end) in cutoffs {
            if !(proven.contains(&(bytes, end)) || bytes == 0 && end == 0) {
                return Err(Reason::Incomplete);
            }
        }
    }
    if target.host == Host::Claude && !claude_owner {
        facts.uncertain = true;
    }
    // Retained inherited marker alone proves no own compaction count. An
    // approval reviewer's such file is a partial copy of the reviewed
    // conversation, whose earlier compactions were left out.
    if inherited_marker && facts.events.is_empty() {
        if reviewer_snapshot {
            return Err(Reason::Snapshot);
        }
        facts.uncertain = true;
    }
    Ok((facts, lineage))
}
/// Split a copied-history fork's compactions, `order` as recorded, into
/// those copied from the conversation `origin` it names and its own. The
/// copied ones are every compaction recorded in a file named for `origin`
/// whose saved header is that conversation's own: they must be one unbroken
/// run at the start of `order`, and none after it may be one of them.
/// Every compaction of `origin`'s history must be in those files: an origin
/// that is itself a fork, or whose files continue a rollout not among them,
/// may hold its history elsewhere, and is not followed. Anything else, or no
/// such conversation, cannot be split. Returns the origin, the files read
/// for it, and how many of `order` are copied.
fn copied_split(
    home: &Path,
    origin: Option<&str>,
    native: &str,
    order: &[String],
    work: &mut Work<'_>,
) -> Result<(String, Vec<Revision>, usize), Reason> {
    // Only a refusal that can change on its own, such as an origin still
    // being written, is kept as itself.
    let settled = |why: Reason| match why {
        Reason::Limit
        | Reason::Cancelled
        | Reason::Replaced
        | Reason::Unreadable
        | Reason::Incomplete => why,
        _ => Reason::CopiedHistory,
    };
    let origin = origin
        .filter(|origin| full_thread_id(origin) && *origin != native)
        .ok_or(Reason::CopiedHistory)?;
    let paths = codex_files(home, origin, work).map_err(settled)?;
    if paths.is_empty() {
        return Err(Reason::CopiedHistory);
    }
    let mut recorded = BTreeSet::new();
    let mut read = Vec::new();
    let mut rollouts = BTreeSet::new();
    let mut bases = Vec::new();
    for path in &paths {
        work.check()?;
        let observed = regular(path).map_err(settled)?;
        let (mut source, identity) = open_regular(path, &observed).map_err(|_| Reason::Replaced)?;
        if !identity.known {
            return Err(Reason::CopiedHistory);
        }
        if identity.len > MAX_BATCH.saturating_sub(work.bytes) {
            return Err(Reason::Limit);
        }
        let row = jsonl::Rows::new(&mut source, identity.len, work)
            .header()
            .map_err(settled)?;
        if row["type"] != "session_meta" || row["payload"]["id"].as_str() != Some(origin) {
            return Err(Reason::CopiedHistory);
        }
        // History outside these files: another conversation's, or a page
        // of this one that is not here.
        if !row["payload"]["forked_from_id"].is_null() {
            return Err(Reason::CopiedHistory);
        }
        match &row["payload"]["history_base"] {
            Value::Null => {}
            base => bases.push(
                base["thread_id"]
                    .as_str()
                    .ok_or(Reason::CopiedHistory)?
                    .to_owned(),
            ),
        }
        let (_, rollout, _) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(super::claude_launch::group::census_name)
            .ok_or(Reason::CopiedHistory)?;
        rollouts.insert(rollout);
        source
            .seek(SeekFrom::Start(0))
            .map_err(|_| Reason::Unreadable)?;
        let mut lines = 0u64;
        let mut rows = jsonl::Rows::new(&mut source, identity.len, work);
        while let Some((row, _)) = rows.next(true).map_err(settled)? {
            lines += 1;
            if lines > MAX_EVENTS as u64 {
                return Err(Reason::Limit);
            }
            if row["type"] == "compacted" {
                let payload = &row["payload"];
                if let Some(id) = payload["compaction_response_id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .or_else(|| payload["window_id"].as_str().filter(|id| !id.is_empty()))
                    .filter(|id| id.len() <= 256)
                {
                    recorded.insert(id.to_owned());
                    if recorded.len() > MAX_EVENTS {
                        return Err(Reason::Limit);
                    }
                }
            }
        }
        verify_jsonl(&source, path, &identity)?;
        read.push(Revision(vec![(path.clone(), Some(identity))]));
    }
    if bases.iter().any(|base| !rollouts.contains(base)) {
        return Err(Reason::CopiedHistory);
    }
    let copied = order.iter().take_while(|id| recorded.contains(*id)).count();
    if order[copied..].iter().any(|id| recorded.contains(id)) {
        return Err(Reason::CopiedHistory);
    }
    Ok((origin.to_owned(), read, copied))
}
/// The compactions a fork inherits: those recorded in each referenced
/// rollout before the cutoff its successor names, following the chain to an
/// original rollout as the reader's `codex_history.references` does. Each
/// referenced rollout is the one file named with its rollout identity,
/// headed as that rollout of the conversation the step allows: at first the
/// conversation the fork names, then the referenced rollout's own and, when
/// it is itself a fork, the one it names. Only bytes below the cutoff are
/// read, ending on a line boundary after the row with the ordinal just before
/// the cutoff, as the reader's `prefix_usage` proves; what was appended after
/// the fork is never read. As there, rows below a spawned thread's inherited
/// context boundary are not its own. Each file read is recorded in `read`.
fn inherited(
    home: &Path,
    fork: &Fork,
    own: &Facts,
    read: &mut Vec<(String, Revision)>,
    work: &mut Work<'_>,
) -> Result<u32, Reason> {
    let mut seen = fork.own.clone();
    let mut allowed = BTreeSet::from([fork.parent.clone()]);
    let mut next = Some(fork.base.clone());
    let mut facts = Facts::default();
    while let Some((rollout, cutoff, end)) = next.take() {
        work.check()?;
        if !seen.insert(rollout.clone()) {
            return Err(Reason::Incomplete);
        }
        if read.len() >= MAX_REFERENCES {
            return Err(Reason::Limit);
        }
        let (path, thread) = referenced(home, &rollout, work)?;
        if !allowed.contains(&thread) {
            return Err(Reason::Ownership);
        }
        let observed = regular(&path)?;
        let (mut source, identity) =
            open_regular(&path, &observed).map_err(|_| Reason::Replaced)?;
        if !identity.known {
            return Err(Reason::Unsupported);
        }
        if cutoff > identity.len {
            return Err(Reason::Incomplete);
        }
        let row = jsonl::Rows::new(&mut source, identity.len, work).header()?;
        let header = codex_header(&row, &path, &thread)?;
        if !header.paginated || header.rollout != rollout {
            return Err(Reason::Unsupported);
        }
        source
            .seek(SeekFrom::Start(0))
            .map_err(|_| Reason::Unreadable)?;
        let owned_from = header.boundary.unwrap_or(0);
        let mut expected = header.first;
        let mut lines = 0u64;
        let mut rows = jsonl::Rows::new(&mut source, cutoff, work);
        while let Some((row, _)) = rows.next(true)? {
            lines += 1;
            if lines > MAX_EVENTS as u64 {
                return Err(Reason::Limit);
            }
            let ordinal = row["ordinal"].as_u64();
            if ordinal.is_none() || ordinal != expected {
                return Err(Reason::Incomplete);
            }
            expected = ordinal.and_then(|ordinal| ordinal.checked_add(1));
            let own = ordinal.is_some_and(|ordinal| ordinal >= owned_from);
            // As for the thread's own count: one header, then its own rows.
            if lines > 1 && row["type"] == "session_meta" && own {
                return Err(Reason::IdentityMismatch);
            }
            if row["type"] == "compacted" && own {
                let payload = &row["payload"];
                let id = payload["compaction_response_id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .or_else(|| payload["window_id"].as_str().filter(|id| !id.is_empty()))
                    .ok_or(Reason::Ownership)?;
                if id.len() > 256 {
                    return Err(Reason::Limit);
                }
                if ownership_metadata(payload, &thread) {
                    facts.uncertain = true;
                }
                // Counted only: the lane's ticks are the fork's own.
                facts.count(id.to_owned(), None);
                if facts.events.len() > MAX_EVENTS {
                    return Err(Reason::Limit);
                }
            }
        }
        // A complete prefix ends at the cutoff after the ordinal before it.
        let complete = if cutoff == 0 {
            end == 0
        } else {
            expected == Some(end)
        };
        if !complete {
            return Err(Reason::Incomplete);
        }
        verify_jsonl(&source, &path, &identity)?;
        read.push((rollout, Revision(vec![(path, Some(identity))])));
        next = header
            .base
            .as_ref()
            .map(|base| {
                Some((
                    base["thread_id"].as_str()?.to_owned(),
                    base["end_byte_offset"].as_u64()?,
                    base["end_ordinal_exclusive"].as_u64()?,
                ))
            })
            .map(|base| base.ok_or(Reason::Incomplete))
            .transpose()?;
        allowed = BTreeSet::from([thread]);
        allowed.extend(header.fork);
    }
    // The same recorded compaction on both sides cannot be told apart.
    if facts.uncertain || facts.events.keys().any(|id| own.events.contains_key(id)) {
        return Err(Reason::Ownership);
    }
    u32::try_from(facts.events.len()).map_err(|_| Reason::Limit)
}
enum CompactionProofError {
    Stopped(Reason),
    Io,
}
impl From<std::io::Error> for CompactionProofError {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}
fn verify_jsonl(source: &fs::File, path: &Path, identity: &FileIdentity) -> Result<(), Reason> {
    let after = source.metadata().map_err(|_| Reason::Unreadable)?;
    let named = regular(path).map_err(|_| Reason::Replaced)?;
    if !unchanged(&FileIdentity::of(&after), identity)
        || !unchanged(&FileIdentity::of(&named), identity)
    {
        return Err(Reason::Replaced);
    }
    Ok(())
}
fn regular(path: &Path) -> Result<fs::Metadata, Reason> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Reason::Missing
        } else {
            Reason::Unreadable
        }
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Reason::IdentityMismatch);
    }
    Ok(metadata)
}
fn revision(path: &Path) -> Result<Revision, Reason> {
    let mut paths = vec![path.to_path_buf()];
    if matches!(
        path.file_name().and_then(|v| v.to_str()),
        Some("store.db" | "state.vscdb")
    ) {
        paths.push(PathBuf::from(format!("{}-wal", path.display())));
        paths.push(PathBuf::from(format!("{}-journal", path.display())));
        if path.ends_with("store.db") {
            paths.push(path.with_file_name("meta.json"));
        }
    }
    paths
        .into_iter()
        .map(|path| {
            let identity = match regular(&path) {
                Ok(metadata) => {
                    let identity = FileIdentity::of(&metadata);
                    if !identity.known {
                        return Err(Reason::Unsupported);
                    }
                    Some(identity)
                }
                Err(Reason::Missing) => None,
                Err(why) => return Err(why),
            };
            Ok((path, identity))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Revision)
}

/// No aliases in the native home or below it. An explicit alternate native
/// home is supported; native source roots that move while reading are refused.
fn safe(path: &Path, home: &Path) -> bool {
    if !path.is_absolute()
        || !path.starts_with(home)
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return false;
    }
    let mut next = Some(path);
    while let Some(current) = next {
        if fs::symlink_metadata(current).is_ok_and(|m| m.file_type().is_symlink()) {
            return false;
        }
        next = current.parent();
    }
    true
}
fn list(path: &Path) -> Result<Vec<PathBuf>, Reason> {
    match fs::read_dir(path) {
        Ok(entries) => entries
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|_| Reason::Unreadable)
            })
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err(Reason::Unreadable),
    }
}
/// Checked disposable SQLite copy, including WAL/journal. SQLite is allowed
/// to recover only the copy, never open the original or create original SHM.
fn snapshot(
    path: &Path,
    revision: &Revision,
    work: &mut Work<'_>,
) -> Result<tempfile::TempDir, Reason> {
    let total: u64 = revision
        .0
        .iter()
        .filter_map(|(_, id)| id.as_ref())
        .map(|id| id.len)
        .sum();
    if total > MAX_SNAPSHOT {
        return Err(Reason::Limit);
    }
    work.charge(total)?;
    let dir = tempfile::Builder::new()
        .prefix("xtrace-compaction-snapshot-")
        .tempdir()
        .map_err(|_| Reason::Unreadable)?;
    for (source, expected) in &revision.0 {
        let Some(expected) = expected else {
            continue;
        };
        let observed = regular(source)?;
        if !unchanged(&FileIdentity::of(&observed), expected) {
            return Err(Reason::Replaced);
        }
        let (mut input, identity) =
            open_regular(source, &observed).map_err(|_| Reason::Replaced)?;
        if !unchanged(&identity, expected) {
            return Err(Reason::Replaced);
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(
                dir.path()
                    .join(source.file_name().ok_or(Reason::IdentityMismatch)?),
            )
            .map_err(|_| Reason::Unreadable)?;
        let mut bytes = [0u8; 256 * 1024];
        let mut remaining = expected.len;
        while remaining > 0 {
            work.check()?;
            let want = bytes.len().min(remaining as usize);
            let read = input
                .read(&mut bytes[..want])
                .map_err(|_| Reason::Unreadable)?;
            if read == 0 {
                return Err(Reason::Replaced);
            }
            output
                .write_all(&bytes[..read])
                .map_err(|_| Reason::Unreadable)?;
            remaining -= read as u64;
        }
        if !unchanged(
            &FileIdentity::of(&input.metadata().map_err(|_| Reason::Unreadable)?),
            expected,
        ) {
            return Err(Reason::Replaced);
        }
    }
    if *revision != self::revision(path)? {
        return Err(Reason::Replaced);
    }
    Ok(dir)
}
fn text(bytes: &[u8]) -> Result<Value, Reason> {
    serde_json::from_slice(bytes).map_err(|_| Reason::Unsupported)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(value: &str) -> Result<Vec<u8>, Reason> {
    if !value.len().is_multiple_of(2) || !value.is_ascii() {
        return Err(Reason::Unsupported);
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|v| {
            u8::from_str_radix(std::str::from_utf8(v).map_err(|_| Reason::Unsupported)?, 16)
                .map_err(|_| Reason::Unsupported)
        })
        .collect()
}
fn hash_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn query_blob(
    db: &Connection,
    ide: bool,
    key: &str,
    work: &mut Work<'_>,
) -> Result<Vec<u8>, Reason> {
    work.check()?;
    let sql = if ide {
        "SELECT value FROM cursorDiskKV WHERE key=?1 AND length(CAST(value AS BLOB))<=?2"
    } else {
        "SELECT data FROM blobs WHERE id=?1 AND length(CAST(data AS BLOB))<=?2"
    };
    let key = if ide {
        format!("agentKv:blob:{key}")
    } else {
        key.to_owned()
    };
    let value: Option<Vec<u8>> = db
        .query_row(
            sql,
            rusqlite::params![key, MAX_SQL_BYTES],
            |row| match row.get_ref(0)? {
                rusqlite::types::ValueRef::Blob(v) | rusqlite::types::ValueRef::Text(v) => {
                    Ok(v.to_vec())
                }
                _ => Err(rusqlite::Error::InvalidQuery),
            },
        )
        .optional()
        .map_err(|_| Reason::Unsupported)?;
    let value = value.ok_or(Reason::Incomplete)?;
    work.decoded = work.decoded.saturating_add(value.len());
    work.nodes += 1;
    if work.decoded > MAX_DECODED || work.nodes > MAX_NODES {
        return Err(Reason::Limit);
    }
    Ok(value)
}
fn varint(bytes: &[u8], at: &mut usize) -> Result<u64, Reason> {
    let mut value = 0u64;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes.get(*at).ok_or(Reason::Incomplete)?;
        *at += 1;
        if shift == 63 && byte > 1 {
            return Err(Reason::Unsupported);
        }
        value |= u64::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return Ok(value);
        }
    }
    Err(Reason::Unsupported)
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CursorNode {
    Root,
    Messages,
    Archive,
    Summary,
}
// Only the reviewed fields of each known node type are followed. In
// particular an archive's summarized_messages are not summary event claims.
fn links(bytes: &[u8], node: CursorNode) -> Result<Vec<(String, CursorNode)>, Reason> {
    let mut at = 0;
    let mut links = Vec::new();
    while at < bytes.len() {
        let tag = varint(bytes, &mut at)?;
        if tag >> 3 == 0 {
            return Err(Reason::Unsupported);
        }
        // The optional legacy summary_archive is a bytes pointer too, but
        // this release does not decode that history. Empty means absent.
        let legacy_archive = node == CursorNode::Root && tag >> 3 == 11;
        if legacy_archive && tag & 7 != 2 {
            return Err(Reason::Unsupported);
        }
        let child = match (node, tag >> 3) {
            (CursorNode::Root | CursorNode::Messages, 1) => Some(CursorNode::Messages),
            (CursorNode::Root, 13) => Some(CursorNode::Archive),
            (CursorNode::Archive, 4) => Some(CursorNode::Summary),
            _ => None,
        };
        if child.is_some() && tag & 7 != 2 {
            return Err(Reason::Unsupported);
        }
        match tag & 7 {
            0 => {
                varint(bytes, &mut at)?;
            }
            1 => at += 8,
            5 => at += 4,
            2 => {
                let length = usize::try_from(varint(bytes, &mut at)?).map_err(|_| Reason::Limit)?;
                let end = at
                    .checked_add(length)
                    .filter(|end| *end <= bytes.len())
                    .ok_or(Reason::Incomplete)?;
                if legacy_archive && length != 0 {
                    return Err(Reason::Unsupported);
                }
                if let Some(child) = child {
                    if length != 32 {
                        return Err(Reason::Unsupported);
                    }
                    if links.len() >= MAX_NODES {
                        return Err(Reason::Limit);
                    }
                    links.push((hex(&bytes[at..end]), child));
                }
                at = end;
            }
            _ => return Err(Reason::Unsupported),
        }
        if at > bytes.len() {
            return Err(Reason::Incomplete);
        }
    }
    if node == CursorNode::Archive && links.len() != 1 {
        return Err(Reason::Incomplete);
    }
    Ok(links)
}
fn walk(
    db: &Connection,
    ide: bool,
    root: &[u8],
    native: &str,
    work: &mut Work<'_>,
) -> Result<Facts, Reason> {
    let mut queue: Vec<(String, CursorNode, usize, bool)> = links(root, CursorNode::Root)?
        .into_iter()
        .rev()
        .map(|(id, node)| (id, node, 1, false))
        .collect();
    let mut active = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut facts = Facts::default();
    while let Some((id, node, depth, exit)) = queue.pop() {
        work.check()?;
        if exit {
            active.remove(&id);
            continue;
        }
        if active.contains(&id) {
            return Err(Reason::Incomplete);
        }
        // A current message visited first cannot bypass the stricter check
        // when an archive also names it as its generated summary.
        if !seen.insert((id.clone(), node)) {
            continue;
        }
        if depth > MAX_DEPTH || !hash_id(&id) {
            return Err(Reason::Limit);
        }
        let bytes = query_blob(db, ide, &id, work)?;
        if hex(&Sha256::digest(&bytes)) != id {
            return Err(Reason::Incomplete);
        }
        if bytes.first() == Some(&b'{') {
            if node == CursorNode::Archive {
                return Err(Reason::Unsupported);
            }
            let leaf = text(&bytes)?;
            if node == CursorNode::Summary
                && (leaf["role"] != "user"
                    || leaf["providerOptions"]["cursor"]["isSummary"] != true)
            {
                return Err(Reason::Ownership);
            }
            if leaf["providerOptions"]["cursor"]["isSummary"] == true
                && ownership_metadata(&leaf, native)
            {
                facts.uncertain = true;
            }
            if leaf["role"] == "user" && leaf["providerOptions"]["cursor"]["isSummary"] == true {
                // Cursor's saved summaries carry no time: counted only.
                facts.count(id, None);
            }
        } else {
            if node == CursorNode::Summary {
                return Err(Reason::Unsupported);
            }
            active.insert(id.clone());
            queue.push((id, node, depth, true));
            let children = links(&bytes, node)?;
            if queue.len().saturating_add(children.len()) > MAX_NODES {
                return Err(Reason::Limit);
            }
            for (child, node) in children.into_iter().rev() {
                queue.push((child, node, depth + 1, false));
            }
        }
    }
    Ok(facts)
}
fn cursor_target(
    db: &Connection,
    dir: &Path,
    path: &Path,
    native: &str,
    work: &mut Work<'_>,
) -> Result<Facts, Reason> {
    if path.ends_with("state.vscdb") {
        work.check()?;
        let key = format!("composerData:{native}");
        // Bound bytes in SQLite before copying either TEXT or BLOB into Rust.
        let raw = db
            .query_row(
                "SELECT CASE WHEN length(CAST(value AS BLOB))<=?2 THEN value END,
                        length(CAST(value AS BLOB)) FROM cursorDiskKV WHERE key=?1",
                rusqlite::params![key, MAX_SQL_BYTES],
                |row| {
                    let bytes: Option<i64> = row.get(1)?;
                    if bytes.is_some_and(|bytes| bytes > MAX_SQL_BYTES) {
                        return Ok(Err(Reason::Limit));
                    }
                    Ok(match row.get_ref(0)? {
                        rusqlite::types::ValueRef::Text(v) | rusqlite::types::ValueRef::Blob(v) => {
                            Ok(v.to_vec())
                        }
                        _ => Err(Reason::Unsupported),
                    })
                },
            )
            .optional()
            .map_err(|_| Reason::Unsupported)?;
        let raw = raw.ok_or(Reason::Missing)??;
        work.decoded = work.decoded.saturating_add(raw.len());
        if work.decoded > MAX_DECODED {
            return Err(Reason::Limit);
        }
        let value = text(&raw)?;
        if value["composerId"].as_str().is_some_and(|id| id != native) {
            return Err(Reason::IdentityMismatch);
        }
        if ownership_metadata(&value, native) {
            return Err(Reason::Ownership);
        }
        let state = value["conversationState"]
            .as_str()
            .and_then(|state| state.strip_prefix('~'))
            .ok_or(Reason::Unsupported)?;
        let root = STANDARD.decode(state).map_err(|_| Reason::Unsupported)?;
        work.decoded = work.decoded.saturating_add(root.len());
        if work.decoded > MAX_DECODED {
            return Err(Reason::Limit);
        }
        walk(db, true, &root, native, work)
    } else {
        if fs::metadata(dir.join("meta.json"))
            .map_err(|_| Reason::Incomplete)?
            .len()
            > 1024 * 1024
        {
            return Err(Reason::Limit);
        }
        let meta = fs::read(dir.join("meta.json")).map_err(|_| Reason::Incomplete)?;
        if meta.len() > 1024 * 1024 {
            return Err(Reason::Limit);
        }
        let meta = text(&meta)?;
        if meta["schemaVersion"] != 1 {
            return Err(Reason::Unsupported);
        }
        if ownership_metadata(&meta, native) {
            return Err(Reason::Ownership);
        }
        let mut statement = db
            .prepare("SELECT value FROM meta WHERE length(value)<=1048576 LIMIT 17")
            .map_err(|_| Reason::Unsupported)?;
        let rows = statement
            .query_map([], |row| match row.get_ref(0)? {
                rusqlite::types::ValueRef::Blob(v) | rusqlite::types::ValueRef::Text(v) => {
                    Ok(v.to_vec())
                }
                _ => Err(rusqlite::Error::InvalidQuery),
            })
            .map_err(|_| Reason::Unsupported)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Reason::Unsupported)?;
        if rows.len() > 16 {
            return Err(Reason::Limit);
        }
        let mut roots = BTreeSet::new();
        for raw in rows {
            let value = text(&raw).or_else(|_| {
                let raw = unhex(std::str::from_utf8(&raw).map_err(|_| Reason::Unsupported)?)?;
                text(&raw)
            })?;
            if ownership_metadata(&value, native) {
                return Err(Reason::Ownership);
            }
            if let Some(root) = value["latestRootBlobId"].as_str() {
                roots.insert(root.to_owned());
            }
        }
        if roots.len() != 1 {
            return Err(Reason::Incomplete);
        }
        let id = roots.into_iter().next().expect("one root");
        if !hash_id(&id) {
            return Err(Reason::Unsupported);
        }
        let root = query_blob(db, false, &id, work)?;
        if hex(&Sha256::digest(&root)) != id {
            return Err(Reason::Incomplete);
        }
        walk(db, false, &root, native, work)
    }
}
