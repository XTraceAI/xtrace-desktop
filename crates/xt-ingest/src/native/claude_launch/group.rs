//! Which physical history files are one Codex thread's own, and whether they
//! form one valid history.
//!
//! The index records only a thread's original rollout; the continuations of
//! a paginated history (`rollout-<ts>-<thread>_<rollout>.jsonl`) are found by
//! a name-only census of the anchored Codex history root, taken once and
//! shared by every thread. The census walks real directories only, never an
//! alias, within fixed bounds, and keeps its place across passes: a pass that
//! runs out of time continues it next time rather than starting again. No
//! group is taken from a census that has not finished.
//!
//! A group is then validated as the reviewed reader's `codex_history.plan`
//! does, from each file's opening header: every header is this thread's own
//! `session_meta`; a flat history is its one original file; a paginated one
//! has exactly one original, unique rollout identities, each continuation's
//! `history_base` naming another file of the group with no cycle, each
//! header's ordinal equal to its base's `end_ordinal_exclusive`, and each
//! base's cutoff a line boundary whose row has the ordinal just before it.
//! A history's first rollout carries the thread's own name. A fork's first
//! rollout has a `history_base` naming a rollout of the conversation it was
//! forked from, outside this group, and a `forked_from_id` naming that other
//! conversation (as the reader's `codex_history._fork_origin`); the
//! referenced history is not this thread's and is not read here. A fork with
//! no history reference and no inherited-context marker, whose saved file may
//! hold copied history, is refused. Rows below an explicit
//! `subagent_history_start_ordinal` are inherited context; a spawned thread
//! or approval reviewer without one inherited nothing, as the reader's
//! `codex_history.context_boundary` has it.
//! Rewinds and abandoned tails are not refused: every file's rows are the
//! thread's own. Every indexed locator of the thread, if the index holds it,
//! must be one of the census's files.

use super::rows;
use super::source::{self, Allowance, BackRead, Budget, LineRead, Reserved, Unread};
use crate::native::session_titles::{
    SpawnCheck, TypedSpawn, codex_contained, codex_segment, full_thread_id, spawn_check,
    typed_spawn,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use xt_store::claude_launch::SegmentGeneration;

/// The most files a census looks at, of any name.
pub(super) const MAX_CENSUS_ENTRIES: usize = 2_000_000;
/// How deep below a history root a census looks: `YYYY/MM/DD/<file>`.
const MAX_DEPTH: usize = 4;
/// The most physical files one thread's history may have.
pub(super) const MAX_SEGMENTS: usize = 16;

/// One history file the census found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CensusFile {
    pub path: PathBuf,
    /// The file's own rollout identity: the thread's for its original name.
    pub rollout: String,
    /// Named as the thread's original rollout (no continuation suffix).
    pub original_name: bool,
}

/// A name-only census of the Codex history roots, taken over as many passes
/// as it needs, one directory entry at a time.
#[derive(Debug)]
pub(super) struct Census {
    roots: Vec<PathBuf>,
    pending: Vec<(PathBuf, usize)>,
    /// The directory being listed, at its depth, where its listing is.
    listing: Option<(std::fs::ReadDir, usize)>,
    groups: BTreeMap<String, Vec<CensusFile>>,
    entries: usize,
    saturated: bool,
    /// A directory or entry could not be read: nothing it covers is known.
    failed: bool,
}

/// The thread, rollout identity and whether it is the original, that a
/// history file's name gives it.
pub(in crate::native) fn census_name(name: &str) -> Option<(String, String, bool)> {
    let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    let thread_of = |head: &str| -> Option<String> {
        let at = head.len().checked_sub(36)?;
        let thread = head.get(at..)?;
        (at > 0 && head.as_bytes()[at - 1] == b'-' && full_thread_id(thread))
            .then(|| thread.to_owned())
    };
    if let Some((head, rollout)) = stem.rsplit_once('_')
        && full_thread_id(rollout)
    {
        let thread = thread_of(head)?;
        return Some((thread, rollout.to_owned(), false));
    }
    let thread = thread_of(stem)?;
    Some((thread.clone(), thread, true))
}

impl Census {
    /// A census of the Codex history roots under `home`, not yet walked.
    pub fn start(roots: &[PathBuf]) -> Self {
        let mut anchored: Vec<PathBuf> = Vec::new();
        for root in roots {
            if let Ok(resolved) = root.canonicalize()
                && std::fs::metadata(&resolved).is_ok_and(|meta| meta.is_dir())
                && !anchored.contains(&resolved)
            {
                anchored.push(resolved);
            }
        }
        Self {
            pending: anchored.iter().map(|root| (root.clone(), 0)).collect(),
            listing: None,
            roots: anchored,
            groups: BTreeMap::new(),
            entries: 0,
            saturated: false,
            failed: false,
        }
    }

    /// Walk on, asking `stop` before each directory entry, until done or it
    /// says to stop; the listing in progress is kept for the next step.
    /// Whether it is done.
    pub fn step(&mut self, stop: &mut dyn FnMut() -> bool) -> bool {
        loop {
            if self.listing.is_none() {
                let Some((directory, depth)) = self.pending.pop() else {
                    return true;
                };
                match std::fs::read_dir(&directory) {
                    Ok(listing) => self.listing = Some((listing, depth)),
                    Err(_) => {
                        // An unreadable directory could hold a file of any
                        // group.
                        self.failed = true;
                        continue;
                    }
                }
            }
            if stop() {
                return false;
            }
            let (listing, depth) = self.listing.as_mut().expect("listing");
            let depth = *depth;
            let Some(entry) = listing.next() else {
                self.listing = None;
                continue;
            };
            let Ok(entry) = entry else {
                self.failed = true;
                continue;
            };
            self.entries += 1;
            if self.entries > MAX_CENSUS_ENTRIES {
                self.saturated = true;
                self.pending.clear();
                self.listing = None;
                return true;
            }
            let Ok(kind) = entry.file_type() else {
                self.failed = true;
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if depth + 1 < MAX_DEPTH {
                    self.pending.push((path, depth + 1));
                }
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if let Some((thread, rollout, original_name)) = census_name(&name) {
                // An alias named as a rollout is kept, so the group it claims
                // is refused rather than read without it.
                self.groups.entry(thread).or_default().push(CensusFile {
                    path,
                    rollout,
                    original_name,
                });
            }
        }
    }

    pub fn complete(&self) -> bool {
        self.pending.is_empty() && self.listing.is_none()
    }

    /// Directory entries looked at so far.
    pub fn walked(&self) -> usize {
        self.entries
    }

    /// Complete, and it looked at everything: only then is a group whole.
    pub fn usable(&self) -> bool {
        self.complete() && !self.failed && !self.saturated
    }

    /// Complete but not whole: an unreadable entry or too many entries. It
    /// is taken again later.
    pub fn broken(&self) -> bool {
        self.complete() && (self.failed || self.saturated)
    }

    /// The roots it walked, as anchored.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Every thread with a history file, in order.
    pub fn threads(&self) -> impl Iterator<Item = &str> {
        self.groups.keys().map(String::as_str)
    }

    /// The files of one thread's history, when the census is complete and
    /// whole.
    pub fn group(&self, thread: &str) -> Option<&[CensusFile]> {
        if !self.usable() {
            return None;
        }
        self.groups.get(thread).map(Vec::as_slice)
    }
}

/// Why a thread's history cannot be read for launches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// No census file, or too many, or an indexed locator the census did not
    /// find.
    Membership,
    /// A file is an alias, outside the root, or could not be read now.
    Source(Unread),
    /// The history's opening lines need more than the thread's allowance.
    OverLimit,
    /// A header is not this thread's own, or the group is not one valid
    /// history.
    Structure,
    /// A fork whose header marks none of its rows as copied: its file may
    /// hold the history it was forked from, so this version reads no launch
    /// of it. Explicitly unsupported, not a failed read.
    Copied,
}

impl Refusal {
    /// What the refusal means for a display check waiting on this history.
    pub fn outcome(self) -> super::ReadOutcome {
        match self {
            Self::Copied => super::ReadOutcome::Unsupported,
            Self::Membership | Self::Source(_) | Self::OverLimit | Self::Structure => {
                super::ReadOutcome::Failed
            }
        }
    }
}

/// One file of a validated history, as the generation its header was read
/// from: the file is read, and its facts are used, only as that generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Segment {
    pub path: PathBuf,
    pub rollout: String,
    pub header: Header,
    pub generation: SegmentGeneration,
}

impl Segment {
    /// Bytes its strings hold.
    pub fn bytes(&self) -> usize {
        self.path.capacity()
            + self.rollout.capacity()
            + self
                .header
                .base
                .as_ref()
                .map_or(0, |(base, _, _)| base.capacity())
    }
}

/// Bytes a list of segments holds, at its capacity.
pub(super) fn segments_bytes(segments: &Vec<Segment>) -> usize {
    segments.capacity() * std::mem::size_of::<Segment>()
        + segments.iter().map(Segment::bytes).sum::<usize>()
}

/// What a file's opening header says, as far as launches need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::native) struct Header {
    pub paginated: bool,
    /// The header row's own ordinal.
    pub ordinal: Option<i64>,
    /// The file this one continues, and where: rollout, byte and ordinal.
    pub base: Option<(String, u64, i64)>,
    /// Rows below this ordinal are inherited context.
    pub inherited_below: Option<i64>,
    /// It names a different conversation it was forked from, and any fork
    /// cutoff it states is its base's: it can start a forked history when its
    /// base is not of this thread.
    pub forked: bool,
    /// Whose thread the header says this is.
    pub role: Role,
    /// It names a conversation it was forked from with neither a history
    /// reference nor an inherited-context marker: nothing in it says which of
    /// its rows were copied from that conversation. Launches never read it.
    pub copied: bool,
}

/// Whose thread a header says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::native) enum Role {
    /// A person's own session, or one that names no other thread.
    Own,
    /// A helper thread spawned by its typed parent.
    Spawned,
    /// An approval reviewer (`guardian`) of the thread it names.
    Reviewer,
}

/// An approval reviewer's header, as the reader's
/// `codex_witness._guardian_root` checks it: exactly
/// `{"subagent": {"other": "guardian"}}` from a `guardian_review`, naming the
/// reviewed thread as its parent and its spawn tree's root as its session,
/// each a full thread identity other than this thread. `None` when the header
/// does not say it is a reviewer; `Some(false)` when it says so but does not
/// fit.
fn reviewer(payload: &serde_json::Map<String, Value>, thread: &str) -> Option<bool> {
    let source = payload.get("source")?;
    if *source != serde_json::json!({"subagent": {"other": "guardian"}}) {
        return None;
    }
    let other = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|id| full_thread_id(id) && id != thread)
    };
    Some(
        payload.get("thread_source").and_then(Value::as_str) == Some("guardian_review")
            && other("parent_thread_id")
            && other("session_id"),
    )
}

fn integer(value: Option<&Value>) -> Result<Option<i64>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_i64().filter(|n| *n >= 0).map(Some).ok_or(()),
    }
}

/// Read a history file's opening line as the thread `thread`'s header. The
/// caller reserves [`source::decoded_bound`] of it first.
pub(in crate::native) fn header(line: &[u8], thread: &str) -> Result<Header, ()> {
    let value: Value = serde_json::from_slice(line).map_err(|_| ())?;
    // The row's structure must decode as every other row's does.
    rows::row(line).ok_or(())?;
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Err(());
    }
    let payload = value.get("payload").and_then(Value::as_object).ok_or(())?;
    if payload.get("id").and_then(Value::as_str) != Some(thread) {
        return Err(());
    }
    let role = match typed_spawn(payload, thread) {
        TypedSpawn::None => match reviewer(payload, thread) {
            Some(true) => Role::Reviewer,
            Some(false) => return Err(()),
            None => {
                if !payload
                    .get("session_id")
                    .is_none_or(|session| session.is_null() || session.as_str() == Some(thread))
                {
                    return Err(());
                }
                Role::Own
            }
        },
        TypedSpawn::Parent(parent)
            if spawn_check(payload, thread, parent) == SpawnCheck::Corroborated =>
        {
            Role::Spawned
        }
        _ => return Err(()),
    };
    let paginated = match payload.get("history_mode") {
        None | Some(Value::Null) => false,
        Some(mode) if mode.as_str() == Some("paginated") => true,
        Some(_) => return Err(()),
    };
    let base = match payload.get("history_base") {
        None | Some(Value::Null) => None,
        Some(Value::Object(base)) => {
            let rollout = base.get("thread_id").and_then(Value::as_str).ok_or(())?;
            let end = base
                .get("end_byte_offset")
                .and_then(Value::as_u64)
                .ok_or(())?;
            let ordinal = integer(base.get("end_ordinal_exclusive"))?.ok_or(())?;
            if !full_thread_id(rollout) {
                return Err(());
            }
            Some((rollout.to_owned(), end, ordinal))
        }
        Some(_) => return Err(()),
    };
    let inherited_below = integer(payload.get("subagent_history_start_ordinal"))?;
    let ordinal = integer(value.get("ordinal"))?;
    // A spawned thread or reviewer with no marker inherited nothing: its file
    // starts with its own task.
    if base.is_some() && !paginated {
        return Err(());
    }
    if inherited_below.is_some() && ordinal.is_none() {
        return Err(());
    }
    // A fork's own rows are told apart by a history reference or, for a
    // spawned helper, by its inherited-context marker: with neither, its file
    // may hold the copied history. A marker on any other fork with no
    // reference is not one Codex writes, and is refused.
    let (forked, copied) = match (payload.get("forked_from_id"), &base) {
        (None | Some(Value::Null), _) => (false, false),
        (Some(parent), Some((_, _, cutoff))) => {
            let stated = integer(payload.get("forked_from_ordinal_exclusive"));
            let forked = parent
                .as_str()
                .is_some_and(|parent| full_thread_id(parent) && parent != thread)
                && stated.is_ok_and(|stated| stated.is_none_or(|stated| stated == *cutoff));
            (forked, false)
        }
        (Some(parent), None) => {
            if !parent.is_string() {
                return Err(());
            }
            match (role, inherited_below) {
                (_, None) => (false, true),
                (Role::Spawned, Some(_)) => (false, false),
                (Role::Own | Role::Reviewer, Some(_)) => return Err(()),
            }
        }
    };
    Ok(Header {
        paginated,
        ordinal,
        base,
        inherited_below,
        forked,
        role,
        copied,
    })
}

/// Validating one thread's whole history from its files' opening lines,
/// over as many passes as the budget needs. Membership and containment are
/// checked from names and stats first, before anything is read. Each file's
/// generation is the one its header was read from; a base cutoff is checked
/// only in that generation of its file.
#[derive(Debug)]
pub(super) struct Planner {
    thread: String,
    files: Vec<CensusFile>,
    allowance: Allowance,
    max_line: usize,
    headers: Vec<(Header, SegmentGeneration)>,
    reading: Option<(std::fs::File, SegmentGeneration, LineRead)>,
    segments: Vec<Segment>,
    /// The next segment whose base cutoff is checked, and its read.
    lookback: usize,
    looking: Option<(std::fs::File, BackRead)>,
    /// What the planner keeps: its files, headers and segments.
    held: Reserved,
}

/// A validated plan: the files in name order, and their reservation.
#[derive(Debug)]
pub(super) struct Plan {
    pub segments: Vec<Segment>,
    pub held: Reserved,
}

fn census_bytes(files: &Vec<CensusFile>) -> usize {
    files.capacity() * std::mem::size_of::<CensusFile>()
        + files
            .iter()
            .map(|file| file.path.capacity() + file.rollout.capacity())
            .sum::<usize>()
}

impl Planner {
    /// Check the thread's census files against the locators the index
    /// recorded for it, by names and stats only. A thread the index holds
    /// (`held`) must have locators; one it does not hold has none, and its
    /// census files alone are its history.
    pub fn start(
        thread: &str,
        files: &[CensusFile],
        indexed: &[PathBuf],
        held: bool,
        roots: &[PathBuf],
        allowance: &Allowance,
        max_line: usize,
    ) -> Result<Self, Refusal> {
        if files.is_empty() || files.len() > MAX_SEGMENTS || (held && indexed.is_empty()) {
            return Err(Refusal::Membership);
        }
        // What the planner keeps, reserved before it is copied: the files,
        // a header each, and the segments made of them.
        let copies = std::mem::size_of_val(files)
            + files
                .iter()
                .map(|file| file.path.as_os_str().len() + file.rollout.len())
                .sum::<usize>();
        let mut held = allowance.reserve(copies).map_err(|_| Refusal::OverLimit)?;
        // Read in name order, which is the files' date order.
        let mut files = files.to_vec();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        held.resize(census_bytes(&files))
            .map_err(|_| Refusal::OverLimit)?;
        let mut identities = Vec::new();
        let mut rollouts = BTreeSet::new();
        for file in &files {
            if !codex_contained(&file.path, roots, thread)
                || codex_segment(&file.path, thread).is_none()
            {
                return Err(Refusal::Source(Unread::Alias));
            }
            if !rollouts.insert(file.rollout.clone()) {
                return Err(Refusal::Structure);
            }
            let now = source::stat(&file.path).map_err(Refusal::Source)?;
            identities.push((now.device, now.inode));
        }
        // Every recorded locator is one of the census's files.
        for path in indexed {
            let now = source::stat(path).map_err(|_| Refusal::Membership)?;
            if !identities.contains(&(now.device, now.inode)) {
                return Err(Refusal::Membership);
            }
        }
        Ok(Self {
            thread: thread.to_owned(),
            files,
            allowance: allowance.clone(),
            max_line,
            headers: Vec::new(),
            reading: None,
            segments: Vec::new(),
            lookback: 0,
            looking: None,
            held,
        })
    }

    /// The bytes the planner actually keeps apart from its fixed-size parts.
    #[cfg(test)]
    pub fn actual(&self) -> usize {
        census_bytes(&self.files)
            + self.headers.capacity() * std::mem::size_of::<(Header, SegmentGeneration)>()
            + self
                .headers
                .iter()
                .map(|(header, _)| {
                    header
                        .base
                        .as_ref()
                        .map_or(0, |(base, _, _)| base.capacity())
                })
                .sum::<usize>()
            + segments_bytes(&self.segments)
            + self
                .reading
                .as_ref()
                .map_or(0, |(_, _, read)| read.actual())
            + self.looking.as_ref().map_or(0, |(_, read)| read.actual())
    }

    /// The generation of every file whose content was read so far, by
    /// rollout: each header read and the one being read.
    pub fn observed(&self) -> Vec<(String, SegmentGeneration)> {
        self.files
            .iter()
            .zip(
                self.headers
                    .iter()
                    .map(|(_, generation)| *generation)
                    .chain(self.reading.as_ref().map(|(_, generation, _)| *generation)),
            )
            .map(|(file, generation)| (file.rollout.clone(), generation))
            .collect()
    }

    fn unread(error: Unread) -> Refusal {
        match error {
            Unread::Memory => Refusal::OverLimit,
            Unread::TooLarge => Refusal::Structure,
            other => Refusal::Source(other),
        }
    }

    /// Read on within `budget`: `None` when paused, else the validated files
    /// in name order, each with the generation its header was read from.
    pub fn advance(&mut self, budget: &mut Budget<'_>) -> Result<Option<Plan>, Refusal> {
        // Every file's opening line.
        while self.headers.len() < self.files.len() {
            let file = &self.files[self.headers.len()];
            if self.reading.is_none() {
                let (handle, generation) = source::open(&file.path).map_err(Refusal::Source)?;
                let read =
                    LineRead::new(0, self.max_line, &self.allowance).map_err(Self::unread)?;
                self.reading = Some((handle, generation, read));
            }
            let (handle, generation, read) = self.reading.as_mut().expect("reading");
            let generation = *generation;
            let Some(line) = read.advance(handle, budget).map_err(Self::unread)? else {
                return Ok(None);
            };
            let parsed = {
                let _decoding = self
                    .allowance
                    .reserve(source::decoded_bound(&line) + rows::row_bound(&line))
                    .map_err(|_| Refusal::OverLimit)?;
                header(&line, &self.thread).map_err(|()| Refusal::Structure)?
            };
            // Copied history is never read as this thread's launches.
            if parsed.copied {
                return Err(Refusal::Copied);
            }
            drop(line);
            self.reading = None;
            let base = parsed.base.as_ref().map_or(0, |(base, _, _)| base.len());
            self.held.grow(base).map_err(|_| Refusal::OverLimit)?;
            source::grow_items(&mut self.headers, &mut self.held)
                .map_err(|_| Refusal::OverLimit)?;
            self.headers.push((parsed, generation));
        }
        if self.segments.is_empty() {
            self.segments = self.structure()?;
        }
        // Each base cutoff is a line boundary after the row with the ordinal
        // just before its continuation's first.
        while self.lookback < self.segments.len() {
            let Some((base, cutoff, ordinal)) = self.segments[self.lookback].header.base.clone()
            else {
                self.lookback += 1;
                continue;
            };
            // A fork's first rollout continues another conversation's
            // history, which is not this thread's and is not read for it.
            if !self.segments.iter().any(|segment| segment.rollout == base) {
                self.lookback += 1;
                continue;
            }
            if cutoff == 0 {
                if ordinal != 0 {
                    return Err(Refusal::Structure);
                }
                self.lookback += 1;
                continue;
            }
            if self.looking.is_none() {
                let base = self
                    .segments
                    .iter()
                    .find(|segment| segment.rollout == base)
                    .ok_or(Refusal::Structure)?;
                let (handle, generation) = source::open(&base.path).map_err(Refusal::Source)?;
                // The base as its header was read, or the plan starts again.
                if generation != base.generation {
                    return Err(Refusal::Source(Unread::Changed));
                }
                if cutoff > u64::try_from(generation.length).unwrap_or(0) {
                    return Err(Refusal::Structure);
                }
                let read =
                    BackRead::new(cutoff, self.max_line, &self.allowance).map_err(Self::unread)?;
                self.looking = Some((handle, read));
            }
            let (handle, read) = self.looking.as_mut().expect("looking");
            let line = match read.advance(handle, budget) {
                Ok(Some(line)) => line,
                Ok(None) => return Ok(None),
                Err(Unread::Memory) => return Err(Refusal::OverLimit),
                Err(_) => return Err(Refusal::Structure),
            };
            let row = {
                let _decoding = self
                    .allowance
                    .reserve(rows::row_bound(&line))
                    .map_err(|_| Refusal::OverLimit)?;
                rows::row(&line).ok_or(Refusal::Structure)?
            };
            drop(line);
            self.looking = None;
            if row.ordinal != Some(ordinal - 1) {
                return Err(Refusal::Structure);
            }
            self.lookback += 1;
        }
        // Only the segments are kept from here on.
        let segments = std::mem::take(&mut self.segments);
        self.files = Vec::new();
        self.headers = Vec::new();
        let mut held = std::mem::replace(
            &mut self.held,
            self.allowance.reserve(0).map_err(|_| Refusal::OverLimit)?,
        );
        held.resize(segments_bytes(&segments))
            .map_err(|_| Refusal::OverLimit)?;
        Ok(Some(Plan { segments, held }))
    }

    /// The history's structure from its headers alone.
    fn structure(&mut self) -> Result<Vec<Segment>, Refusal> {
        let thread = self.thread.as_str();
        // The segments copy the files and headers: reserved first.
        let copies = self.files.len() * std::mem::size_of::<Segment>()
            + self
                .files
                .iter()
                .zip(&self.headers)
                .map(|(file, (header, _))| {
                    file.path.as_os_str().len()
                        + file.rollout.len()
                        + header.base.as_ref().map_or(0, |(base, _, _)| base.len())
                })
                .sum::<usize>();
        self.held.grow(copies).map_err(|_| Refusal::OverLimit)?;
        let segments: Vec<Segment> = self
            .files
            .iter()
            .zip(&self.headers)
            .map(|(file, (header, generation))| Segment {
                path: file.path.clone(),
                rollout: file.rollout.clone(),
                header: header.clone(),
                generation: *generation,
            })
            .collect();
        let paginated = segments[0].header.paginated;
        if segments.iter().any(|s| s.header.paginated != paginated) {
            return Err(Refusal::Structure);
        }
        if !paginated {
            // A flat history is its one original file.
            let [only] = segments.as_slice() else {
                return Err(Refusal::Structure);
            };
            if !self.files[0].original_name || only.rollout != thread {
                return Err(Refusal::Structure);
            }
            return Ok(segments);
        }
        // One first rollout, the thread's own name: an original, or a fork's
        // whose base is not of this group; every other base in the group;
        // header ordinals at their base's cutoff; no cycle.
        let by_rollout: BTreeMap<&str, &Segment> =
            segments.iter().map(|s| (s.rollout.as_str(), s)).collect();
        let first = |segment: &Segment| match &segment.header.base {
            None => true,
            Some((base, _, _)) => segment.header.forked && !by_rollout.contains_key(base.as_str()),
        };
        let firsts: Vec<&Segment> = segments.iter().filter(|s| first(s)).collect();
        if !matches!(firsts.as_slice(), [only] if only.rollout == thread) {
            return Err(Refusal::Structure);
        }
        for segment in &segments {
            let start = segment
                .header
                .base
                .as_ref()
                .map_or(0, |(_, _, ordinal)| *ordinal);
            if segment.header.ordinal != Some(start) {
                return Err(Refusal::Structure);
            }
            let mut seen = BTreeSet::from([segment.rollout.as_str()]);
            let mut at = segment;
            while let Some((base, _, _)) = at.header.base.as_ref().filter(|_| !first(at)) {
                let next = by_rollout.get(base.as_str()).ok_or(Refusal::Structure)?;
                if !seen.insert(next.rollout.as_str()) {
                    return Err(Refusal::Structure);
                }
                at = next;
            }
        }
        Ok(segments)
    }
}

/// The locators the index recorded for the Codex thread `thread`, as paths.
pub(super) fn indexed_paths(
    store: &xt_store::Store,
    thread: &str,
) -> xt_store::Result<Vec<PathBuf>> {
    Ok(
        crate::native::session_titles::title_sources(store, xt_store::Host::Codex, thread)?
            .into_iter()
            .filter_map(|source| {
                source
                    .locator
                    .strip_prefix("codex:")
                    .map(|path| Path::new(path).to_path_buf())
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: &str = "01a00000-0000-7000-8000-0000000000aa";
    const NEXT: &str = "01a00000-0000-7000-8000-0000000000b1";

    /// The launch scan reads a spawned header as every other reader does: by
    /// its typed parent, whatever root `session_id` it shares. An explicit
    /// parent that disagrees, or no root other than the thread, is not the
    /// thread's own header.
    #[test]
    fn a_nested_spawn_header_is_the_threads_own() {
        const PARENT: &str = "01a00000-0000-7000-8000-0000000000c1";
        const ROOT: &str = "01a00000-0000-7000-8000-0000000000c2";
        let line = |session: Option<&str>, explicit: Option<&str>| {
            let mut value = serde_json::json!({"type": "session_meta", "ordinal": 0,
                "payload": {"id": THREAD, "history_mode": "paginated",
                            "subagent_history_start_ordinal": 1,
                            "source": {"subagent": {"thread_spawn": {
                                "parent_thread_id": PARENT, "depth": 2}}}}});
            if let Some(session) = session {
                value["payload"]["session_id"] = session.into();
            }
            if let Some(explicit) = explicit {
                value["payload"]["parent_thread_id"] = explicit.into();
            }
            value.to_string().into_bytes()
        };
        for (session, explicit, own) in [
            (Some(PARENT), None, true),
            (Some(PARENT), Some(PARENT), true),
            (Some(ROOT), Some(PARENT), true),
            (Some(ROOT), None, true),
            (Some(ROOT), Some(ROOT), false),
            (Some(PARENT), Some(ROOT), false),
            (None, Some(PARENT), false),
            (Some(THREAD), Some(PARENT), false),
        ] {
            assert_eq!(
                header(&line(session, explicit), THREAD).is_ok(),
                own,
                "{session:?} {explicit:?}"
            );
        }
    }

    /// A fork's first header names a different conversation and states the
    /// cutoff of its history reference, if it states one; one with no
    /// history reference is marked copied, as its file may hold copied
    /// history.
    #[test]
    fn a_fork_header_needs_a_reference_and_a_consistent_origin() {
        const PARENT: &str = "01a00000-0000-7000-8000-0000000000c1";
        let line = |forked: Value, cutoff: Value, base: bool| {
            let mut value = serde_json::json!({"type": "session_meta", "ordinal": 7,
                "payload": {"id": THREAD, "session_id": THREAD, "history_mode": "paginated",
                            "forked_from_id": forked, "forked_from_ordinal_exclusive": cutoff}});
            if base {
                value["payload"]["history_base"] = serde_json::json!(
                    {"thread_id": PARENT, "end_byte_offset": 10, "end_ordinal_exclusive": 7});
            }
            value.to_string().into_bytes()
        };
        let forked = |forked: Value, cutoff: Value, base: bool| {
            header(&line(forked, cutoff, base), THREAD).map(|header| (header.forked, header.copied))
        };
        assert_eq!(forked(PARENT.into(), 7.into(), true), Ok((true, false)));
        assert_eq!(forked(PARENT.into(), Value::Null, true), Ok((true, false)));
        assert_eq!(forked(Value::Null, Value::Null, true), Ok((false, false)));
        // Not a fork origin it can vouch for: never a first rollout.
        assert_eq!(forked(PARENT.into(), 6.into(), true), Ok((false, false)));
        assert_eq!(forked(THREAD.into(), 7.into(), true), Ok((false, false)));
        assert_eq!(
            forked("not-a-thread".into(), 7.into(), true),
            Ok((false, false))
        );
        assert_eq!(forked(PARENT.into(), (-1).into(), true), Ok((false, false)));
        // No history reference: its rows may be copied, whatever thread it
        // names; a name that is not text is refused.
        assert_eq!(forked(PARENT.into(), 7.into(), false), Ok((false, true)));
        assert_eq!(
            forked("not-a-thread".into(), Value::Null, false),
            Ok((false, true))
        );
        assert_eq!(forked(7.into(), Value::Null, false), Err(()));
        assert_eq!(
            forked(serde_json::json!([PARENT]), Value::Null, false),
            Err(())
        );
        assert_eq!(forked(Value::Null, Value::Null, false), Ok((false, false)));
    }

    /// A spawned helper thread with no inherited-context marker inherited
    /// nothing. With a marker, one that names a conversation it was forked
    /// from without a history reference is still read: the marker says which
    /// rows are copied. Without one, it is copied history.
    #[test]
    fn a_spawned_header_needs_no_marker_and_a_marked_fork_is_not_copied() {
        const PARENT: &str = "01a00000-0000-7000-8000-0000000000c1";
        let line = |marker: Option<i64>, forked: bool| {
            let mut value = serde_json::json!({"type": "session_meta", "ordinal": 0,
                "payload": {"id": THREAD, "session_id": PARENT, "history_mode": "paginated",
                            "source": {"subagent": {"thread_spawn": {
                                "parent_thread_id": PARENT, "depth": 1}}}}});
            if let Some(marker) = marker {
                value["payload"]["subagent_history_start_ordinal"] = marker.into();
            }
            if forked {
                value["payload"]["forked_from_id"] = PARENT.into();
            }
            header(&value.to_string().into_bytes(), THREAD)
                .map(|header| (header.role, header.inherited_below, header.copied))
        };
        assert_eq!(line(None, false), Ok((Role::Spawned, None, false)));
        assert_eq!(line(Some(5), false), Ok((Role::Spawned, Some(5), false)));
        assert_eq!(line(Some(5), true), Ok((Role::Spawned, Some(5), false)));
        assert_eq!(line(None, true), Ok((Role::Spawned, None, true)));
        // A person's own conversation with a marker and a fork with no
        // reference is not a shape Codex writes: refused.
        let own = serde_json::json!({"type": "session_meta", "ordinal": 0,
            "payload": {"id": THREAD, "session_id": THREAD, "history_mode": "paginated",
                        "subagent_history_start_ordinal": 5, "forked_from_id": PARENT}});
        assert_eq!(header(&own.to_string().into_bytes(), THREAD), Err(()));
    }

    /// An approval reviewer names the reviewed thread as its parent and its
    /// spawn tree's root as its session; one that does not fit, or names
    /// itself, is not this thread's header.
    #[test]
    fn a_reviewer_header_names_a_different_parent_and_root() {
        const PARENT: &str = "01a00000-0000-7000-8000-0000000000c1";
        const ROOT: &str = "01a00000-0000-7000-8000-0000000000c2";
        let line = |edit: &dyn Fn(&mut Value)| {
            let mut value = serde_json::json!({"type": "session_meta", "ordinal": 0,
                "payload": {"id": THREAD, "session_id": ROOT, "parent_thread_id": PARENT,
                            "history_mode": "paginated", "thread_source": "guardian_review",
                            "subagent_history_start_ordinal": 3,
                            "source": {"subagent": {"other": "guardian"}}}});
            edit(&mut value);
            header(&value.to_string().into_bytes(), THREAD)
                .map(|header| (header.role, header.inherited_below))
        };
        assert_eq!(line(&|_| {}), Ok((Role::Reviewer, Some(3))));
        assert_eq!(
            line(&|v| v["payload"]["session_id"] = PARENT.into()),
            Ok((Role::Reviewer, Some(3)))
        );
        assert_eq!(
            line(&|v| v["payload"]["subagent_history_start_ordinal"] = Value::Null),
            Ok((Role::Reviewer, None))
        );
        let refused: [(&str, Value); 7] = [
            // A marked reviewer that also names a fork with no reference.
            ("/payload/forked_from_id", PARENT.into()),
            ("/payload/session_id", THREAD.into()),
            ("/payload/parent_thread_id", THREAD.into()),
            ("/payload/parent_thread_id", Value::Null),
            ("/payload/session_id", Value::Null),
            ("/payload/thread_source", "user".into()),
            ("/payload/source/subagent/depth", 1.into()),
        ];
        for (at, value) in refused {
            let edit = |v: &mut Value| {
                let (parent, key) = at.rsplit_once('/').unwrap();
                v.pointer_mut(parent).unwrap()[key] = value.clone();
            };
            assert_eq!(line(&edit), Err(()), "{at} {value}");
        }
    }

    /// The first rollout of a paginated history carries the thread's own
    /// name, as the compaction reader requires; a continuation-named file
    /// that starts the history is not one history.
    #[test]
    fn the_first_rollout_carries_the_threads_name() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap().join("sessions");
        let day = root.join("2026/09/07");
        std::fs::create_dir_all(&day).unwrap();
        let header = serde_json::json!({"type": "session_meta", "ordinal": 0,
            "payload": {"id": THREAD, "history_mode": "paginated"}});
        let plan = |name: String, original_name: bool, rollout: &str| {
            let path = day.join(name);
            std::fs::write(&path, format!("{header}\n")).unwrap();
            let files = [CensusFile {
                path: path.clone(),
                rollout: rollout.to_owned(),
                original_name,
            }];
            let allowance = Allowance::new(1 << 20);
            let mut planner = Planner::start(
                THREAD,
                &files,
                &[],
                false,
                std::slice::from_ref(&root),
                &allowance,
                1 << 16,
            )?;
            let mut budget = Budget::new(1 << 20, std::time::Duration::from_secs(10), None);
            let planned = planner.advance(&mut budget);
            std::fs::remove_file(path).unwrap();
            planned.map(|plan| plan.is_some())
        };
        assert_eq!(
            plan(
                format!("rollout-2026-09-07T00-00-00-{THREAD}.jsonl"),
                true,
                THREAD
            ),
            Ok(true)
        );
        assert_eq!(
            plan(
                format!("rollout-2026-09-07T00-00-00-{THREAD}_{NEXT}.jsonl"),
                false,
                NEXT
            ),
            Err(Refusal::Structure)
        );
    }

    #[test]
    fn names_give_the_thread_and_rollout() {
        assert_eq!(
            census_name(&format!("rollout-2026-09-07T19-04-03-{THREAD}.jsonl")),
            Some((THREAD.into(), THREAD.into(), true))
        );
        assert_eq!(
            census_name(&format!(
                "rollout-2026-09-28T16-14-04-{THREAD}_{NEXT}.jsonl"
            )),
            Some((THREAD.into(), NEXT.into(), false))
        );
        for name in [
            format!("rollout-{THREAD}.jsonl"),
            format!("rollout-x-{}.jsonl", THREAD.to_uppercase()),
            format!("rollout-x-{THREAD}.json"),
            format!("x-{THREAD}.jsonl"),
            format!("rollout-x-{THREAD}_{NEXT}x.jsonl"),
        ] {
            assert_eq!(census_name(&name), None, "{name}");
        }
    }

    #[test]
    fn a_census_keeps_its_place_across_stops() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap().join("sessions");
        for (day, name) in [
            (
                "2026/09/07",
                format!("rollout-2026-09-07T00-00-00-{THREAD}.jsonl"),
            ),
            (
                "2026/09/28",
                format!("rollout-2026-09-28T00-00-00-{THREAD}_{NEXT}.jsonl"),
            ),
        ] {
            std::fs::create_dir_all(root.join(day)).unwrap();
            std::fs::write(root.join(day).join(name), b"{}\n").unwrap();
        }
        let mut census = Census::start(&[root]);
        let mut steps = 0;
        // One entry per pass: every pass continues, none starts again.
        while !census.step(&mut {
            let mut asked = 0;
            move || {
                asked += 1;
                asked > 1
            }
        }) {
            assert!(
                census.group(THREAD).is_none(),
                "an incomplete census gives no group"
            );
            steps += 1;
            assert!(steps < 20);
        }
        assert!(steps >= 3, "{steps}");
        let group = census.group(THREAD).unwrap();
        assert_eq!(group.len(), 2);
    }

    /// One large directory: the census looks at a small, fixed number of
    /// entries each step and continues where it stopped, and a stop at once
    /// looks at none.
    #[test]
    fn a_large_directory_is_walked_a_few_entries_at_a_time() {
        const FILES: usize = 2_000;
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap().join("sessions");
        let day = root.join("2026/09/07");
        std::fs::create_dir_all(&day).unwrap();
        for index in 0..FILES {
            let thread = format!("01a00000-0000-7000-8000-{index:012x}");
            std::fs::write(
                day.join(format!("rollout-2026-09-07T00-00-00-{thread}.jsonl")),
                b"{}\n",
            )
            .unwrap();
        }
        let mut census = Census::start(&[root]);
        let mut steps = 0;
        loop {
            let before = census.walked();
            let mut asked = 0;
            // A tiny deterministic allowance: stop after 50 entries.
            let done = census.step(&mut || {
                asked += 1;
                asked > 50
            });
            assert!(
                census.walked() - before <= 51,
                "step {steps} looked at {}",
                census.walked() - before
            );
            steps += 1;
            if done {
                break;
            }
            assert!(steps < 1_000);
        }
        assert!(steps >= FILES / 51, "{steps}");
        assert!(census.usable());
        assert_eq!(census.threads().count(), FILES);
        // A stop at once: nothing is looked at, and the place is kept.
        let mut census = Census::start(&[temp.path().canonicalize().unwrap().join("sessions")]);
        let mut asked = 0;
        census.step(&mut || {
            asked += 1;
            asked > 10
        });
        let before = census.walked();
        assert!((1..=11).contains(&before), "{before}");
        assert!(!census.step(&mut || true));
        assert_eq!(census.walked(), before, "a stop at once looks at nothing");
    }
}
