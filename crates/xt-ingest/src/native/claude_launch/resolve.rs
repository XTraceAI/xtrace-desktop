//! Matching one validated launch with the Claude session it named, over as
//! many passes as its reads need.
//!
//! A check begins only while the thread's published validation is current
//! and the launch's history file is exactly its published generation. Its two
//! recorded lines are read again, charged to the pass's budget, and must
//! still be that launch and the output of its acknowledgment call.
//!
//! The child must be exactly one indexed Claude user session, and its one
//! recorded transcript, streamed in budgeted chunks, must be its own and
//! fresh: every line naming it, none dated before the launch, its first
//! eligible input (a user record that is not meta, sidechain or a tool
//! result, text or not) one text equal to the prompt, appearing once, the
//! unique first input (no other input untimed or at or before it; later
//! inputs are allowed). It may come any time after the launch — before or
//! after the acknowledgment, the job's end or anything else in the parent's
//! history, which it is never compared with. No other project may hold a
//! file of its name, and the transcript must be the same generation at the
//! end as at the start, ending in a complete line. A transcript whose last
//! line has no newline yet is not decided: the check is retried once that
//! generation changes, and not read again before. The projects are listed one
//! entry at a time within the pass's budget, and a listing that fails now is
//! retried. The check's copies of the launch, the prompt, the partial line
//! and the identifiers seen before the first input are reserved from the
//! thread's allowance and dropped with the check. A finished check returns
//! the proof; the caller checks the whole history once more and writes it
//! only through the store's guarded write.

use super::cell::{self, Tool};
use super::command;
use super::rows;
use super::source::{self, Allowance, Budget, LineRead, Lines, Reserved, Unread};
use crate::native::session_titles::{claude_contained, title_sources};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::{Path, PathBuf},
};
use xt_store::{
    Host, Store,
    claude_launch::{CandidateRow, ChildState, MemberRecord, SegmentGeneration},
    creation::{CLAUDE_LAUNCH_CREATE_VERSION, ClaudeLaunchCreationProof},
};

/// What became of one check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    /// The child is not indexed yet.
    Waiting,
    /// A source was busy or changed; a timed pass looks again.
    Retry,
    /// The history is no longer its published validation: validate again.
    Stale,
    /// The child's transcript disagrees with the launch.
    Rejected,
    /// The launch's recorded lines no longer read as that launch.
    SourceRejected,
    /// The thread's allowance cannot hold the check now; it waits its turn.
    Deferred,
    /// The child's transcript ends in a line still being written, and has
    /// not changed since.
    Unfinished,
}

impl Outcome {
    /// The child state this outcome leaves, when it sets one.
    pub fn state(self) -> Option<ChildState> {
        match self {
            Self::Waiting => Some(ChildState::Waiting),
            Self::Retry | Self::Stale | Self::Unfinished => Some(ChildState::Retry),
            Self::Rejected => Some(ChildState::Rejected),
            Self::SourceRejected | Self::Deferred => None,
        }
    }
}

/// What the resolver may read.
pub(super) struct Sources<'a> {
    pub home: &'a Path,
    pub programs: &'a [PathBuf],
    pub max_line: usize,
    pub max_child_bytes: u64,
    pub max_child_lines: usize,
    /// Transcripts found ending in an unfinished line, at that generation.
    pub unfinished: &'a HashMap<PathBuf, SegmentGeneration>,
}

/// Where one check is after a pass.
#[derive(Debug)]
pub(super) enum Resolve {
    /// The pass's budget ended; the check resumes here next pass.
    Paused(Box<Resolution>),
    /// Everything was read: the proof, for the caller's final checks.
    Ready(Box<Ready>),
    Finished(Outcome),
    /// The transcript at this path ends, at this generation, in a line
    /// still being written.
    Unfinished(PathBuf, SegmentGeneration),
}

/// A proof ready to be written, with what it rests on: the launch, the
/// parent's members and the child's transcript as it was read.
#[derive(Debug)]
pub(super) struct Ready {
    pub proof: ClaudeLaunchCreationProof,
    pub row: CandidateRow,
    pub members: Vec<MemberRecord>,
    pub transcript: PathBuf,
    pub transcript_generation: SegmentGeneration,
}

impl Ready {
    /// Whether the child's transcript is still exactly what was read.
    pub fn transcript_current(&self) -> bool {
        source::stat(&self.transcript).ok() == Some(self.transcript_generation)
    }
}

/// The launch's prompt and time, from its line.
#[derive(Debug)]
struct Recorded {
    prompt: String,
    launched_ms: i64,
    _prompt: Reserved,
}

#[derive(Debug)]
enum Phase {
    Lines {
        file: File,
        launch: LineRead,
        acknowledgment: LineRead,
        launch_line: Option<Vec<u8>>,
    },
    Child {
        transcript: PathBuf,
        stream: Box<ChildStream>,
    },
    /// No other project holds a file of the child's name: the projects,
    /// listed one entry at a time.
    Unique {
        transcript: PathBuf,
        /// The transcript's generation its first input was read from.
        generation: SegmentGeneration,
        name: String,
        listing: std::fs::ReadDir,
        first: String,
    },
}

/// One check in progress.
#[derive(Debug)]
pub(super) struct Resolution {
    row: CandidateRow,
    members: Vec<MemberRecord>,
    generation: SegmentGeneration,
    segment: PathBuf,
    child: String,
    parent: String,
    phase: Phase,
    /// Its copies of the launch, members, names and the walk's names.
    held: Reserved,
}

/// Bytes a candidate row's strings hold, at capacity.
pub(super) fn row_bytes(row: &CandidateRow) -> usize {
    let candidate = &row.candidate;
    candidate.key.parent_native_session_id.capacity()
        + candidate.key.rollout_id.capacity()
        + candidate.key.launch_call_id.capacity()
        + candidate.child_native_session_id.capacity()
        + candidate.acknowledgment_call_id.capacity()
        + candidate
            .process_session_id
            .as_ref()
            .map_or(0, String::capacity)
}

/// Bytes a list of member records holds, at capacity.
#[cfg(test)]
pub(super) fn members_bytes(members: &Vec<MemberRecord>) -> usize {
    members.capacity() * std::mem::size_of::<MemberRecord>()
        + members
            .iter()
            .map(|member| member.rollout_id.capacity())
            .sum::<usize>()
}

impl Resolution {
    pub fn row(&self) -> &CandidateRow {
        &self.row
    }

    /// The bytes the check actually keeps apart from its fixed-size parts.
    #[cfg(test)]
    pub fn actual(&self) -> usize {
        let phase = match &self.phase {
            Phase::Lines {
                launch,
                acknowledgment,
                launch_line,
                ..
            } => {
                launch.actual()
                    + acknowledgment.actual()
                    + launch_line.as_ref().map_or(0, Vec::capacity)
            }
            Phase::Child { transcript, stream } => transcript.capacity() + stream.actual(),
            Phase::Unique {
                transcript,
                name,
                first,
                ..
            } => transcript.capacity() + name.capacity() + first.capacity(),
        };
        row_bytes(&self.row)
            + members_bytes(&self.members)
            + self.segment.capacity()
            + self.child.capacity()
            + self.parent.capacity()
            + phase
    }
}

/// Begin checking `row`, whose file `segment` must be exactly `members`'
/// generation for its rollout. Reads nothing yet.
pub(super) fn begin(
    store: &Store,
    row: &CandidateRow,
    segment: &Path,
    members: &[MemberRecord],
    sources: &Sources<'_>,
    allowance: &Allowance,
) -> xt_store::Result<Resolve> {
    let finished = |outcome| Ok(Resolve::Finished(outcome));
    let candidate = &row.candidate;
    let children =
        store.user_sessions_with_native(Host::Claude, &candidate.child_native_session_id)?;
    let child = match children.as_slice() {
        [] => return finished(Outcome::Waiting),
        [one] => one.clone(),
        _ => return finished(Outcome::Rejected),
    };
    let parents =
        store.user_sessions_with_native(Host::Codex, &candidate.key.parent_native_session_id)?;
    let [parent] = parents.as_slice() else {
        return finished(Outcome::Retry);
    };
    // A transcript found unfinished is not read again until it changes.
    if let [transcript] = transcripts(store, &candidate.child_native_session_id)?.as_slice()
        && let Some(generation) = sources.unfinished.get(transcript)
        && source::stat(transcript).ok() == Some(*generation)
    {
        return finished(Outcome::Unfinished);
    }
    let Some(member) = members
        .iter()
        .find(|member| member.rollout_id == candidate.key.rollout_id && member.valid)
    else {
        return finished(Outcome::Stale);
    };
    let Ok((file, now)) = source::open(segment) else {
        return finished(Outcome::Stale);
    };
    if now != member.generation {
        return finished(Outcome::Stale);
    }
    let (Ok(launch_at), Ok(acknowledged_at)) = (
        u64::try_from(candidate.launch_offset),
        u64::try_from(candidate.acknowledgment_offset),
    ) else {
        return finished(Outcome::SourceRejected);
    };
    let (launch, acknowledgment) = match (
        LineRead::new(launch_at, sources.max_line, allowance),
        LineRead::new(acknowledged_at, sources.max_line, allowance),
    ) {
        (Ok(launch), Ok(acknowledgment)) => (launch, acknowledgment),
        _ => return finished(Outcome::Deferred),
    };
    // The check's copies, reserved before they are made.
    let copies = std::mem::size_of::<Resolution>()
        + row_bytes(row)
        + std::mem::size_of_val(members)
        + members
            .iter()
            .map(|member| member.rollout_id.len())
            .sum::<usize>()
        + segment.as_os_str().len()
        + child.capacity()
        + parent.len();
    let Ok(held) = allowance.reserve(copies) else {
        return finished(Outcome::Deferred);
    };
    Ok(Resolve::Paused(Box::new(Resolution {
        row: row.clone(),
        members: members.to_vec(),
        generation: member.generation,
        segment: segment.to_path_buf(),
        child,
        parent: parent.clone(),
        phase: Phase::Lines {
            file,
            launch,
            acknowledgment,
            launch_line: None,
        },
        held,
    })))
}

/// The child's recorded transcripts.
fn transcripts(store: &Store, child: &str) -> xt_store::Result<Vec<PathBuf>> {
    Ok(title_sources(store, Host::Claude, child)?
        .into_iter()
        .filter_map(|source| source.locator.strip_prefix("claude:").map(PathBuf::from))
        .collect())
}

/// Continue a check within `budget`.
pub(super) fn resume(
    store: &Store,
    mut resolution: Box<Resolution>,
    sources: &Sources<'_>,
    allowance: &Allowance,
    budget: &mut Budget<'_>,
) -> xt_store::Result<Resolve> {
    let finished = |outcome| Ok(Resolve::Finished(outcome));
    if let Phase::Lines {
        file,
        launch,
        acknowledgment,
        launch_line,
    } = &mut resolution.phase
    {
        if launch_line.is_none() {
            match launch.advance(file, budget) {
                Ok(Some(line)) => *launch_line = Some(line),
                Ok(None) => return Ok(Resolve::Paused(resolution)),
                Err(Unread::Memory) => return finished(Outcome::Deferred),
                Err(_) => return finished(Outcome::Stale),
            }
        }
        let acknowledgment_line = match acknowledgment.advance(file, budget) {
            Ok(Some(line)) => line,
            Ok(None) => return Ok(Resolve::Paused(resolution)),
            Err(Unread::Memory) => return finished(Outcome::Deferred),
            Err(_) => return finished(Outcome::Stale),
        };
        // Still exactly the published generation.
        if source::generation_of(file).ok() != Some(resolution.generation)
            || source::stat(&resolution.segment).ok() != Some(resolution.generation)
        {
            return finished(Outcome::Stale);
        }
        let launch_line = launch_line.take().expect("read");
        let recorded = match recorded(
            &resolution.row,
            &launch_line,
            &acknowledgment_line,
            sources,
            allowance,
        ) {
            Ok(recorded) => recorded,
            Err(outcome) => return finished(outcome),
        };
        drop(launch_line);
        drop(acknowledgment_line);
        // The child's one recorded transcript.
        let child_native = resolution.row.candidate.child_native_session_id.clone();
        let mut transcripts = transcripts(store, &child_native)?;
        if transcripts.len() != 1 {
            return finished(if transcripts.is_empty() {
                Outcome::Retry
            } else {
                Outcome::Rejected
            });
        }
        let transcript = transcripts.remove(0);
        let projects = sources.home.join(".claude/projects");
        let mut roots = vec![projects.clone()];
        if let Ok(resolved) = projects.canonicalize()
            && resolved != projects
        {
            roots.push(resolved);
        }
        if !claude_contained(&transcript, &roots, &child_native) {
            return finished(Outcome::Rejected);
        }
        if resolution.held.grow(transcript.as_os_str().len()).is_err() {
            return finished(Outcome::Deferred);
        }
        let stream =
            match ChildStream::open(&transcript, &child_native, recorded, sources, allowance) {
                Ok(stream) => stream,
                Err(Unread::TooLarge | Unread::Alias) => return finished(Outcome::Rejected),
                Err(Unread::Memory) => return finished(Outcome::Deferred),
                Err(_) => return finished(Outcome::Retry),
            };
        resolution.phase = Phase::Child {
            transcript,
            stream: Box::new(stream),
        };
    }
    if let Phase::Child { transcript, stream } = &mut resolution.phase {
        let first = match stream.advance(budget) {
            Ok(None) => return Ok(Resolve::Paused(resolution)),
            Ok(Some(Ok(first))) => first,
            Ok(Some(Err(()))) => return finished(Outcome::Rejected),
            Err(Unread::Unfinished) => {
                return Ok(Resolve::Unfinished(
                    std::mem::take(transcript),
                    stream.generation,
                ));
            }
            Err(Unread::TooLarge | Unread::Alias) => return finished(Outcome::Rejected),
            Err(Unread::Memory) => return finished(Outcome::Deferred),
            Err(_) => return finished(Outcome::Retry),
        };
        // The projects the transcript's own belongs to, listed next.
        let Some(projects) = transcript.parent().and_then(Path::parent) else {
            return finished(Outcome::Rejected);
        };
        let Ok(listing) = std::fs::read_dir(projects) else {
            return finished(Outcome::Retry);
        };
        let name = format!("{}.jsonl", resolution.row.candidate.child_native_session_id);
        if resolution.held.grow(name.len() + first.len()).is_err() {
            return finished(Outcome::Deferred);
        }
        resolution.phase = Phase::Unique {
            transcript: std::mem::take(transcript),
            generation: stream.generation,
            name,
            listing,
            first,
        };
    }
    let Phase::Unique {
        transcript,
        name,
        listing,
        ..
    } = &mut resolution.phase
    else {
        unreachable!("the transcript is read first");
    };
    match sole_transcript(transcript, name, listing, budget) {
        Walk::Paused => return Ok(Resolve::Paused(resolution)),
        Walk::Other => return finished(Outcome::Rejected),
        Walk::Unreadable => return finished(Outcome::Retry),
        Walk::Sole => {}
    }
    let Resolution {
        row,
        members,
        child,
        parent,
        phase:
            Phase::Unique {
                first,
                transcript,
                generation,
                ..
            },
        ..
    } = *resolution
    else {
        unreachable!("unique");
    };
    let candidate = &row.candidate;
    let proof = ClaudeLaunchCreationProof {
        child_session_id: child,
        child_native_session_id: candidate.child_native_session_id.clone(),
        parent_session_id: parent,
        parent_native_session_id: candidate.key.parent_native_session_id.clone(),
        first_record_uuid: first,
        launch_call_id: candidate.key.launch_call_id.clone(),
        launch_operation_index: candidate.key.launch_operation_index,
        process_session_id: candidate.process_session_id.clone(),
        acknowledgment_call_id: candidate.acknowledgment_call_id.clone(),
        acknowledgment_operation_index: candidate.acknowledgment_operation_index,
        segment_rollout_id: candidate.key.rollout_id.clone(),
        launch_ordinal: candidate.launch_ordinal,
        acknowledgment_ordinal: candidate.acknowledgment_ordinal,
        evidence_version: CLAUDE_LAUNCH_CREATE_VERSION,
    };
    Ok(Resolve::Ready(Box::new(Ready {
        proof,
        row,
        members,
        transcript,
        transcript_generation: generation,
    })))
}

/// What the two recorded lines say, when they are still that launch and the
/// output of its acknowledgment call.
fn recorded(
    row: &CandidateRow,
    launch: &[u8],
    acknowledgment: &[u8],
    sources: &Sources<'_>,
    allowance: &Allowance,
) -> Result<Recorded, Outcome> {
    let candidate = &row.candidate;
    let rejected = Outcome::SourceRejected;
    let _decoding = allowance
        .reserve(
            source::decoded_bound(launch)
                .saturating_add(rows::row_bound(launch))
                .saturating_add(rows::row_bound(acknowledgment)),
        )
        .map_err(|_| Outcome::Deferred)?;
    let head = rows::row(launch).ok_or(rejected)?;
    let value: Value = serde_json::from_slice(launch).map_err(|_| rejected)?;
    let Ok(Some(rows::Item::Call { call_id, input, .. })) = rows::item(&value) else {
        return Err(rejected);
    };
    if !head.is_exec()
        || call_id != candidate.key.launch_call_id
        || head.ordinal != candidate.launch_ordinal
    {
        return Err(rejected);
    }
    let ops = cell::parse_operations(&input).map_err(|()| rejected)?;
    let op = ops
        .get(candidate.key.launch_operation_index as usize)
        .filter(|op| op.tool == Tool::ExecCommand)
        .ok_or(rejected)?;
    let submitted = op
        .str("cmd")
        .and_then(|cmd| command::launch(cmd, sources.programs))
        .filter(|launch| launch.child == candidate.child_native_session_id)
        .ok_or(rejected)?;
    let launched_ms = rows::timestamp(&value).ok_or(rejected)?;
    let acknowledged = rows::row(acknowledgment).ok_or(rejected)?;
    if !acknowledged.is_output()
        || acknowledged.call_id() != Some(candidate.acknowledgment_call_id.as_str())
        || acknowledged.ordinal != candidate.acknowledgment_ordinal
    {
        return Err(rejected);
    }
    let reserved = allowance
        .reserve(submitted.prompt.capacity())
        .map_err(|_| Outcome::Deferred)?;
    Ok(Recorded {
        prompt: submitted.prompt,
        launched_ms,
        _prompt: reserved,
    })
}

/// The first eligible input seen so far.
#[derive(Debug)]
struct First {
    uuid: String,
    ts_ms: i64,
    matches: bool,
    /// Lines naming its identifier.
    named: usize,
}

/// A child transcript's stream, and what it has established so far.
#[derive(Debug)]
struct ChildStream {
    file: File,
    path: PathBuf,
    generation: SegmentGeneration,
    lines: Lines,
    native: String,
    recorded: Recorded,
    allowance: Allowance,
    max_lines: usize,
    count: usize,
    /// Record identifiers seen before the first input (few), reserved.
    before_first: HashSet<String>,
    before_reserved: Reserved,
    first: Option<First>,
    broken: bool,
}

/// Record identifiers kept before the first input; past it, the transcript
/// is not one this version reads.
const MAX_BEFORE_FIRST: usize = 10_000;

impl ChildStream {
    #[cfg(test)]
    fn actual(&self) -> usize {
        self.lines.actual()
            + self.path.capacity()
            + self.native.capacity()
            + self.recorded.prompt.capacity()
            + source::table_actual(self.before_first.capacity(), std::mem::size_of::<String>())
            + self
                .before_first
                .iter()
                .map(String::capacity)
                .sum::<usize>()
            + self.first.as_ref().map_or(0, |first| first.uuid.capacity())
    }

    fn open(
        path: &Path,
        native: &str,
        recorded: Recorded,
        sources: &Sources<'_>,
        allowance: &Allowance,
    ) -> Result<Self, Unread> {
        let (file, generation) = source::open(path)?;
        if u64::try_from(generation.length).unwrap_or(u64::MAX) > sources.max_child_bytes {
            return Err(Unread::TooLarge);
        }
        // Its copies of the path and name.
        let before_reserved = allowance.reserve(path.as_os_str().len() + native.len())?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            generation,
            lines: Lines::new(sources.max_line, allowance)?,
            native: native.to_owned(),
            recorded,
            allowance: allowance.clone(),
            max_lines: sources.max_child_lines,
            count: 0,
            before_first: HashSet::new(),
            before_reserved,
            first: None,
            broken: false,
        })
    }

    /// Read on within `budget`: `None` when paused, else the first input's
    /// identifier or `Err(())` when the transcript is not the launch's.
    fn advance(&mut self, budget: &mut Budget<'_>) -> Result<Option<Result<String, ()>>, Unread> {
        let len = u64::try_from(self.generation.length).map_err(|_| Unread::Changed)?;
        while self.lines.next < len && !self.broken {
            let Some(chunk) =
                source::read_chunk(&self.file, self.lines.next, len - self.lines.next, budget)?
            else {
                return Ok(None);
            };
            let Self {
                lines,
                native,
                recorded,
                allowance,
                max_lines,
                count,
                before_first,
                before_reserved,
                first,
                broken,
                ..
            } = self;
            let mut state = LineState {
                native,
                recorded,
                max_lines: *max_lines,
                count,
                before_first,
                before_reserved,
                first,
                broken,
            };
            lines.feed(&chunk, &mut |_, bytes| {
                if bytes.is_empty() {
                    return Ok(());
                }
                let _decoding = allowance.reserve(source::decoded_bound(bytes))?;
                state.line(bytes)
            })?;
        }
        if self.broken {
            return Ok(Some(Err(())));
        }
        let after = source::generation_of(&self.file)?;
        if after != self.generation || source::stat(&self.path)? != self.generation {
            return Err(Unread::Changed);
        }
        // A last line still being written: the prefix decides nothing.
        if !self.lines.finished() {
            return Err(Unread::Unfinished);
        }
        Ok(Some(match &self.first {
            Some(first) if first.matches && first.named == 1 => Ok(first.uuid.clone()),
            _ => Err(()),
        }))
    }
}

/// The stream's state as one line sees it.
struct LineState<'a> {
    native: &'a str,
    recorded: &'a Recorded,
    max_lines: usize,
    count: &'a mut usize,
    before_first: &'a mut HashSet<String>,
    before_reserved: &'a mut Reserved,
    first: &'a mut Option<First>,
    broken: &'a mut bool,
}

impl LineState<'_> {
    fn line(&mut self, line: &[u8]) -> Result<(), Unread> {
        if *self.broken {
            return Ok(());
        }
        *self.count += 1;
        if *self.count > self.max_lines {
            *self.broken = true;
            return Ok(());
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            *self.broken = true;
            return Ok(());
        };
        if !value.is_object() {
            *self.broken = true;
            return Ok(());
        }
        if let Some(session) = value.get("sessionId")
            && session.as_str() != Some(self.native)
        {
            *self.broken = true;
            return Ok(());
        }
        let at = match value.get("timestamp") {
            None => None,
            Some(stamp) => match stamp.as_str().and_then(rows::millis) {
                Some(at) => Some(at),
                None => {
                    *self.broken = true;
                    return Ok(());
                }
            },
        };
        if at.is_some_and(|at| at < self.recorded.launched_ms) {
            *self.broken = true;
            return Ok(());
        }
        let uuid = value.get("uuid").and_then(Value::as_str);
        let eligible = value.get("type").and_then(Value::as_str) == Some("user")
            && value.get("isMeta").and_then(Value::as_bool) != Some(true)
            && value.get("isSidechain").and_then(Value::as_bool) != Some(true)
            && value["message"].get("role").and_then(Value::as_str) == Some("user");
        let text = match &value["message"]["content"] {
            Value::String(text) => Some(Some(text.as_str())),
            Value::Array(parts)
                if parts.iter().any(|part| {
                    part.get("type").and_then(Value::as_str) == Some("tool_result")
                }) =>
            {
                None
            }
            Value::Array(parts) => Some(match parts.as_slice() {
                [part] if part.get("type").and_then(Value::as_str) == Some("text") => {
                    part.get("text").and_then(Value::as_str)
                }
                _ => None,
            }),
            _ => Some(None),
        };
        let input = eligible && text.is_some();
        match self.first {
            Some(first) => {
                if uuid == Some(first.uuid.as_str()) {
                    first.named += 1;
                }
                // The first input is unique as the first, as the store
                // requires: no other input untimed or at or before it. Later
                // inputs are the conversation's.
                if input && at.is_none_or(|at| at <= first.ts_ms) {
                    *self.broken = true;
                }
            }
            None if input => {
                let (Some(uuid), Some(at)) = (uuid, at) else {
                    *self.broken = true;
                    return Ok(());
                };
                self.before_reserved.grow(uuid.len())?;
                *self.first = Some(First {
                    uuid: uuid.to_owned(),
                    ts_ms: at,
                    matches: text.flatten() == Some(self.recorded.prompt.as_str()),
                    named: 1 + usize::from(self.before_first.contains(uuid)),
                });
            }
            None => {
                if let Some(uuid) = uuid
                    && !self.before_first.contains(uuid)
                {
                    if self.before_first.len() >= MAX_BEFORE_FIRST {
                        *self.broken = true;
                        return Ok(());
                    }
                    source::insert_copy(self.before_first, uuid, self.before_reserved)?;
                }
            }
        }
        Ok(())
    }
}

/// Where the walk over the projects is.
#[derive(Debug, PartialEq, Eq)]
enum Walk {
    /// The pass's entries or time ended; the listing is kept.
    Paused,
    /// No project but the transcript's own holds a file of its name.
    Sole,
    /// Another project holds one.
    Other,
    /// A listing or lookup failed now: looked at again later.
    Unreadable,
}

/// Whether any project but the transcript's own holds a file named `name`,
/// one listed entry at a time within `budget`. Only names are looked up;
/// nothing is opened.
fn sole_transcript(
    transcript: &Path,
    name: &str,
    listing: &mut std::fs::ReadDir,
    budget: &mut Budget<'_>,
) -> Walk {
    loop {
        if !budget.entry() {
            return Walk::Paused;
        }
        let entry = match listing.next() {
            None => return Walk::Sole,
            Some(Ok(entry)) => entry,
            Some(Err(_)) => return Walk::Unreadable,
        };
        let named = entry.path().join(name);
        if named == transcript {
            continue;
        }
        match std::fs::symlink_metadata(&named) {
            Ok(_) => return Walk::Other,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) => {}
            Err(_) => return Walk::Unreadable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    const CHILD: &str = "0c000000-0000-4000-8000-0000000000c1";
    const PROMPT: &str = "Synthetic prompt";

    fn user(uuid: &str, at: &str, content: Value) -> String {
        json!({"type": "user", "uuid": uuid, "timestamp": at, "sessionId": CHILD,
            "message": {"role": "user", "content": content}})
        .to_string()
    }

    /// Stream `lines` as a transcript with a tiny budget per pass.
    fn check(lines: &[String]) -> Result<String, ()> {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join(format!("{CHILD}.jsonl"));
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        let at = |text: &str| rows::millis(text).unwrap();
        let unfinished = HashMap::new();
        let sources = Sources {
            home: temp.path(),
            programs: &[],
            max_line: 1 << 20,
            max_child_bytes: 1 << 20,
            max_child_lines: 1000,
            unfinished: &unfinished,
        };
        let allowance = Allowance::new(1 << 20);
        let mut stream = ChildStream::open(
            &path,
            CHILD,
            Recorded {
                prompt: PROMPT.into(),
                launched_ms: at("2026-09-29T07:58:36.333Z"),
                _prompt: allowance.reserve(PROMPT.len()).unwrap(),
            },
            &sources,
            &allowance,
        )
        .unwrap();
        for _ in 0..10_000 {
            let mut budget = Budget::new(64, Duration::from_secs(60), None);
            if let Some(result) = stream.advance(&mut budget).unwrap() {
                return result;
            }
        }
        panic!("never finished");
    }

    /// The first input must be unique as the first: a later input, whatever
    /// its text (an interruption marker as any other), leaves it the first;
    /// another input untimed, at the same time or earlier does not.
    #[test]
    fn a_later_input_leaves_the_first_input_first() {
        let first = user("f", "2026-09-29T07:58:40.536Z", json!(PROMPT));
        for later in [
            user("s", "2026-09-29T08:00:00Z", json!("More")),
            user(
                "i",
                "2026-09-29T08:14:21.100Z",
                json!([{"type": "text", "text": "[Request interrupted by user]"}]),
            ),
            user(
                "a",
                "2026-09-29T07:58:40.537Z",
                json!("Arbitrary later text"),
            ),
        ] {
            assert_eq!(
                check(&[first.clone(), later.clone()]),
                Ok("f".into()),
                "{later}"
            );
        }
        let untimed = json!({"type": "user", "uuid": "u", "sessionId": CHILD,
            "message": {"role": "user", "content": "Untimed"}})
        .to_string();
        for competing in [
            user("e", "2026-09-29T07:58:40.536Z", json!("Same time")),
            user(
                "b",
                "2026-09-29T07:58:40Z",
                json!("Earlier, later in the file"),
            ),
            untimed,
        ] {
            assert_eq!(
                check(&[first.clone(), competing.clone()]),
                Err(()),
                "{competing}"
            );
        }
    }

    /// The first input may come any time after the launch: nothing in the
    /// parent's history bounds it.
    #[test]
    fn the_first_input_must_be_the_prompt_after_the_launch_and_alone() {
        let first = user("f", "2026-09-29T07:58:40.536Z", json!(PROMPT));
        let result = json!({"type": "user", "uuid": "r", "timestamp": "2026-09-29T07:59:00Z",
            "sessionId": CHILD, "message": {"role": "user",
            "content": [{"type": "tool_result", "tool_use_id": "t", "content": "x"}]}})
        .to_string();
        let resumed = user("later", "2026-09-29T08:43:30Z", json!("Resumed request"));
        assert_eq!(
            check(&[first.clone(), result.clone(), resumed.clone()]),
            Ok("f".into())
        );
        assert_eq!(
            check(&[user(
                "f",
                "2026-09-29T07:58:40.536Z",
                json!([{"type": "text", "text": PROMPT}])
            )]),
            Ok("f".into())
        );
        for late in ["2026-09-29T08:20:00Z", "2026-10-09T08:20:00Z"] {
            assert_eq!(
                check(&[user("f", late, json!(PROMPT))]),
                Ok("f".into()),
                "{late}"
            );
        }
        for lines in [
            // Another prompt, an image first.
            vec![user("f", "2026-09-29T07:58:40.536Z", json!("Different"))],
            vec![
                user("i", "2026-09-29T07:58:39Z", json!([{"type": "image"}])),
                first.clone(),
            ],
            // Before the launch, or a line of an older session before it.
            vec![user("f", "2026-09-29T07:58:30Z", json!(PROMPT))],
            vec![
                json!({"type": "summary", "timestamp": "2026-09-28T10:00:00Z",
                    "sessionId": CHILD})
                .to_string(),
                first.clone(),
            ],
            // Another session's line, the record twice, nothing at all.
            vec![
                first.clone(),
                json!({"type": "x", "sessionId": "other"}).to_string(),
            ],
            vec![first.clone(), first.clone()],
            vec![result],
        ] {
            assert_eq!(check(&lines), Err(()), "{lines:?}");
        }
    }
}
