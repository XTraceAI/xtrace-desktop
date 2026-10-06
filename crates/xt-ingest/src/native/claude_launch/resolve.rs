//! Matching one validated launch with the Claude session it named, over as
//! many passes as its reads need.
//!
//! A check begins only while the thread's published validation is current
//! and the launch's history file is exactly its published generation. Its
//! launch and acknowledgment lines, plus the two same-file binding witness
//! lines when used, are read again within the pass budget and must still
//! derive that exact launch.
//!
//! The child must be exactly one indexed Claude user session, and its one
//! recorded transcript, streamed in budgeted chunks, must be its own and
//! fresh: every line naming it, none dated before the launch, its first
//! eligible input (a user record that is not meta, sidechain or a tool
//! result, text or not) one text — equal to the prompt when the command
//! submitted a literal one; a prompt read from a redirected file is never
//! opened, so there the launch's literal `--session-id`, its own
//! acknowledgment and this fresh child bind it — appearing once, the
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
//! thread's allowance and dropped with the check.
//!
//! A `codex exec --json` launch's child is checked instead from its own
//! saved header: the launch's recorded lines are read again and must still be
//! that launch, the launch's follower is replayed over the rows from the
//! launch to its acknowledgment and must acknowledge there with that
//! operation's own result naming exactly this thread (its own position among
//! what the cell emitted, in the launch's output or a wait's), and the child
//! must be exactly one indexed Codex user session whose
//! one recorded original rollout, contained in the Codex history root, opens
//! with its own header: its `source` is `exec`, it was opened after the
//! launch, and it names no parent, fork, history base or inherited rows.
//! Only that opening line is read, and the file must be the same generation
//! after it as before. The child's first input is the index's own first
//! eligible input of that session, dated at or after the launch; until the
//! index holds one the launch waits.
//!
//! Only then is the parent looked up: the thread whose history holds the
//! launch, as exactly one indexed Codex user session. A finished check
//! returns the child fact, which needs no parent, and the relation proof when
//! the parent is found; the caller checks the whole history once more, writes
//! the fact first and the proof only through the store's guarded write.

use super::ack::{Ack, Acknowledged, Broken};
use super::cell::{self, Tool};
use super::group::{self, Role};
use super::rows;
use super::source::{self, Allowance, Budget, LineRead, Lines, Reserved, Unread};
use super::{codex_cli, command};
use crate::native::session_source::host_roots;
use crate::native::session_titles::{
    Segment, claude_contained, codex_contained, codex_segment, title_sources,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::{Path, PathBuf},
};
use xt_store::{
    Host, Store,
    child_fact::{ChildEvidence, ChildFact},
    claude_launch::{CandidateRow, ChildState, MemberRecord, SegmentGeneration},
    creation::{CLAUDE_LAUNCH_CREATE_VERSION, CODEX_CLI_LAUNCH_VERSION, ClaudeLaunchCreationProof},
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
    /// The child's own sources were read and disagree with the launch.
    Rejected,
    /// The child's sources could not be read as one: more than one indexed
    /// session or transcript holds it, its file is an alias, outside its
    /// root or beyond this version's bounds, or another project holds a file
    /// of its name. Nothing was decided; the child it names stays checking.
    Unclear,
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
            Self::SourceRejected | Self::Deferred | Self::Unclear => None,
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
    /// The longest launch window, from a launch row to its acknowledgment.
    pub max_window_bytes: u64,
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

/// A proved child ready to be written, with what it rests on: the launch,
/// the thread's members and the child's transcript as it was read.
#[derive(Debug)]
pub(super) struct Ready {
    /// That the launch created this child, whoever the parent turns out to be.
    pub fact: ChildFact,
    /// The relation, when the thread is exactly one indexed Codex user
    /// session; `None` leaves the child without a parent.
    pub proof: Option<ClaudeLaunchCreationProof>,
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

/// The launch's prompt and time, from its line. No prompt for one read from
/// a redirected file.
#[derive(Debug)]
pub(super) struct Recorded {
    prompt: Option<String>,
    launched_ms: i64,
    _prompt: Reserved,
}

impl Recorded {
    /// What a launch submitted and when, its literal prompt reserved first.
    pub fn new(prompt: &str, launched_ms: i64, allowance: &Allowance) -> Result<Self, Unread> {
        let reserved = allowance.reserve(prompt.len())?;
        Ok(Self {
            prompt: Some(prompt.to_owned()),
            launched_ms,
            _prompt: reserved,
        })
    }
}

/// The first turn of a child transcript, when a check asks for it: the
/// first input's saved directory and time, and the last assistant message
/// before the next input. Only the answer's record, time and an in-memory
/// digest of its text are kept, never the text.
#[derive(Clone, Debug, Default)]
pub(super) struct Turn {
    pub cwd: Option<String>,
    pub first_ms: Option<i64>,
    closed: bool,
    message: Option<TurnMessage>,
}

#[derive(Clone, Debug)]
struct TurnMessage {
    id: Option<String>,
    texts: usize,
    other: bool,
    answer: Option<Answer>,
}

/// The one text of the first turn's final assistant message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Answer {
    pub uuid: String,
    pub ts_ms: i64,
    pub digest: [u8; 32],
}

impl Turn {
    fn first(&mut self, value: &Value, at: Option<i64>) {
        self.cwd = value.get("cwd").and_then(Value::as_str).map(str::to_owned);
        self.first_ms = at;
    }

    fn line(&mut self, value: &Value, at: Option<i64>, input: bool) {
        if self.closed {
            return;
        }
        if input {
            self.closed = true;
            return;
        }
        if value.get("type").and_then(Value::as_str) != Some("assistant")
            || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            return;
        }
        let id = value["message"]
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let same = id.is_some()
            && self
                .message
                .as_ref()
                .is_some_and(|message| message.id == id);
        if !same {
            self.message = Some(TurnMessage {
                id,
                texts: 0,
                other: false,
                answer: None,
            });
        }
        let message = self.message.as_mut().expect("set");
        let uuid = value.get("uuid").and_then(Value::as_str);
        let mut text = |text: &str| {
            use sha2::Digest;
            message.texts += 1;
            message.answer = match (uuid, at) {
                (Some(uuid), Some(ts_ms)) => Some(Answer {
                    uuid: uuid.to_owned(),
                    ts_ms,
                    digest: sha2::Sha256::digest(text.as_bytes()).into(),
                }),
                _ => None,
            };
        };
        match &value["message"]["content"] {
            Value::String(body) => text(body),
            Value::Array(blocks) => {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => match block.get("text").and_then(Value::as_str) {
                            Some(body) => text(body),
                            None => message.other = true,
                        },
                        Some("thinking" | "redacted_thinking") => {}
                        _ => message.other = true,
                    }
                }
            }
            _ => message.other = true,
        }
    }

    /// The first turn's answer: its final assistant message holds exactly
    /// one text and nothing but thinking besides.
    pub fn answer(&self) -> Option<&Answer> {
        self.message
            .as_ref()
            .filter(|message| message.texts == 1 && !message.other)
            .and_then(|message| message.answer.as_ref())
    }
}

#[derive(Debug)]
enum Phase {
    Lines {
        file: File,
        binding_call: Option<LineRead>,
        binding_output: Option<LineRead>,
        launch: LineRead,
        acknowledgment: Box<LineRead>,
        binding_call_line: Option<Vec<u8>>,
        binding_output_line: Option<Vec<u8>>,
        launch_line: Option<Vec<u8>>,
    },
    Child {
        transcript: PathBuf,
        stream: Box<ChildStream>,
    },
    /// A Codex launch: its follower replayed over the parent's rows from
    /// the launch row to its acknowledgment row.
    Replay {
        file: File,
        lines: Lines,
        state: Box<ReplayState>,
        /// Just after the acknowledgment row.
        end: u64,
        launched_ms: i64,
    },
    /// A Codex child: the opening line of its one recorded original
    /// rollout, as the generation it was opened at.
    Header {
        rollout: PathBuf,
        file: File,
        generation: SegmentGeneration,
        read: Box<LineRead>,
        launched_ms: i64,
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

/// What a replayed launch window has established: the launch's follower,
/// where and how it acknowledged, and whether a row ended the replay.
#[derive(Debug)]
pub(super) struct ReplayState {
    ack: Ack,
    acknowledged: Option<(u64, Acknowledged)>,
    broken: bool,
}

/// One check in progress.
#[derive(Debug)]
pub(super) struct Resolution {
    row: CandidateRow,
    members: Vec<MemberRecord>,
    generation: SegmentGeneration,
    segment: PathBuf,
    child: String,
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
                binding_call,
                binding_output,
                launch,
                acknowledgment,
                binding_call_line,
                binding_output_line,
                launch_line,
                ..
            } => {
                binding_call.as_ref().map_or(0, LineRead::actual)
                    + binding_output.as_ref().map_or(0, LineRead::actual)
                    + binding_call_line.as_ref().map_or(0, Vec::capacity)
                    + binding_output_line.as_ref().map_or(0, Vec::capacity)
                    + launch.actual()
                    + acknowledgment.actual()
                    + std::mem::size_of::<LineRead>()
                    + launch_line.as_ref().map_or(0, Vec::capacity)
            }
            Phase::Child { transcript, stream } => transcript.capacity() + stream.actual(),
            Phase::Replay { lines, state, .. } => lines.actual() + state.ack.actual(),
            Phase::Header { rollout, read, .. } => rollout.capacity() + read.actual(),
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
    let children = store
        .user_sessions_with_native(candidate.child_host, &candidate.child_native_session_id)?;
    let child = match children.as_slice() {
        [] => return finished(Outcome::Waiting),
        [one] => one.clone(),
        _ => return finished(Outcome::Unclear),
    };
    // The parent is looked up only once the child is proved.
    // A transcript found unfinished is not read again until it changes.
    if candidate.child_host == Host::Claude
        && let [transcript] = transcripts(store, &candidate.child_native_session_id)?.as_slice()
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
    let (binding_call, binding_output) = match (
        candidate.binding_call_offset,
        candidate.binding_output_offset,
    ) {
        (None, None) => (None, None),
        (Some(call), Some(output))
            if call >= 0 && output > call && output < candidate.launch_offset =>
        {
            let (Ok(call), Ok(output)) = (
                LineRead::new(call as u64, sources.max_line, allowance),
                LineRead::new(output as u64, sources.max_line, allowance),
            ) else {
                return finished(Outcome::Deferred);
            };
            (Some(call), Some(output))
        }
        _ => return finished(Outcome::SourceRejected),
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
        + std::mem::size_of::<LineRead>();
    let Ok(held) = allowance.reserve(copies) else {
        return finished(Outcome::Deferred);
    };
    Ok(Resolve::Paused(Box::new(Resolution {
        row: row.clone(),
        members: members.to_vec(),
        generation: member.generation,
        segment: segment.to_path_buf(),
        child,
        phase: Phase::Lines {
            file,
            binding_call,
            binding_output,
            launch,
            acknowledgment: Box::new(acknowledgment),
            binding_call_line: None,
            binding_output_line: None,
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
    if let Phase::Replay { .. } = &resolution.phase {
        return codex_replay(store, resolution, sources, allowance, budget);
    }
    if let Phase::Header { .. } = &resolution.phase {
        return codex_child(store, resolution, allowance, budget);
    }
    if let Phase::Lines {
        file,
        binding_call,
        binding_output,
        launch,
        acknowledgment,
        binding_call_line,
        binding_output_line,
        launch_line,
    } = &mut resolution.phase
    {
        if let Some(read) = binding_call.as_mut()
            && binding_call_line.is_none()
        {
            match read.advance(file, budget) {
                Ok(Some(line)) => *binding_call_line = Some(line),
                Ok(None) => return Ok(Resolve::Paused(resolution)),
                Err(Unread::Memory) => return finished(Outcome::Deferred),
                Err(_) => return finished(Outcome::Stale),
            }
        }
        if let Some(read) = binding_output.as_mut()
            && binding_output_line.is_none()
        {
            match read.advance(file, budget) {
                Ok(Some(line)) => *binding_output_line = Some(line),
                Ok(None) => return Ok(Resolve::Paused(resolution)),
                Err(Unread::Memory) => return finished(Outcome::Deferred),
                Err(_) => return finished(Outcome::Stale),
            }
        }
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
        // A Codex child is proved from its own saved header.
        if resolution.row.candidate.child_host == Host::Codex {
            let (launched_ms, ops) = match recorded_codex(
                &resolution.row,
                &launch_line,
                &acknowledgment_line,
                binding_call.is_some() || binding_output.is_some(),
                allowance,
            ) {
                Ok(read) => read,
                Err(outcome) => return finished(outcome),
            };
            // The rows after the launch row, through the acknowledgment row,
            // within the scan's own window bound.
            let candidate = &resolution.row.candidate;
            let (Ok(launch_at), Ok(answered_at)) = (
                u64::try_from(candidate.launch_offset),
                u64::try_from(candidate.acknowledgment_offset),
            ) else {
                return finished(Outcome::SourceRejected);
            };
            let start = launch_at + launch_line.len() as u64 + 1;
            let end = answered_at + acknowledgment_line.len() as u64 + 1;
            drop(launch_line);
            drop(acknowledgment_line);
            if start > answered_at || end - start > sources.max_window_bytes {
                return finished(Outcome::SourceRejected);
            }
            let Ok((opened, now)) = source::open(&resolution.segment) else {
                return finished(Outcome::Stale);
            };
            if now != resolution.generation {
                return finished(Outcome::Stale);
            }
            let follower = Ack::new(
                &candidate.key.launch_call_id,
                candidate.key.launch_operation_index as usize,
                ops,
                allowance,
            );
            let (Ok(follower), Ok(mut lines)) = (follower, Lines::new(sources.max_line, allowance))
            else {
                return finished(Outcome::Deferred);
            };
            (lines.next, lines.complete) = (start, start);
            if resolution
                .held
                .grow(std::mem::size_of::<ReplayState>())
                .is_err()
            {
                return finished(Outcome::Deferred);
            }
            resolution.phase = Phase::Replay {
                file: opened,
                lines,
                state: Box::new(ReplayState {
                    ack: follower.reading_thread(),
                    acknowledged: None,
                    broken: false,
                }),
                end,
                launched_ms,
            };
            return codex_replay(store, resolution, sources, allowance, budget);
        }
        let recorded = match recorded(
            &resolution.row,
            &launch_line,
            &acknowledgment_line,
            binding_call_line.as_deref(),
            binding_output_line.as_deref(),
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
                Outcome::Unclear
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
            return finished(Outcome::Unclear);
        }
        if resolution.held.grow(transcript.as_os_str().len()).is_err() {
            return finished(Outcome::Deferred);
        }
        let stream =
            match ChildStream::open(&transcript, &child_native, recorded, sources, allowance) {
                Ok(stream) => stream,
                Err(Unread::TooLarge | Unread::Alias) => return finished(Outcome::Unclear),
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
            Ok(Some(Err(Refused::Mismatch))) => return finished(Outcome::Rejected),
            Ok(Some(Err(Refused::Unclear))) => return finished(Outcome::Unclear),
            Err(Unread::Unfinished) => {
                return Ok(Resolve::Unfinished(
                    std::mem::take(transcript),
                    stream.generation,
                ));
            }
            Err(Unread::TooLarge | Unread::Alias) => return finished(Outcome::Unclear),
            Err(Unread::Memory) => return finished(Outcome::Deferred),
            Err(_) => return finished(Outcome::Retry),
        };
        // The projects the transcript's own belongs to, listed next.
        let Some(projects) = transcript.parent().and_then(Path::parent) else {
            return finished(Outcome::Unclear);
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
        Walk::Other => return finished(Outcome::Unclear),
        Walk::Unreadable => return finished(Outcome::Retry),
        Walk::Sole => {}
    }
    let Resolution {
        row,
        members,
        child,
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
    let fact = ChildFact {
        child_session_id: child.clone(),
        child_host: Host::Claude,
        child_native_session_id: candidate.child_native_session_id.clone(),
        evidence_kind: ChildEvidence::CodexClaudeLaunch,
        evidence_version: CLAUDE_LAUNCH_CREATE_VERSION,
        source_native_session_id: candidate.key.parent_native_session_id.clone(),
        source_rollout_id: Some(candidate.key.rollout_id.clone()),
        launch_call_id: Some(candidate.key.launch_call_id.clone()),
        launch_operation_index: Some(candidate.key.launch_operation_index),
        first_record_uuid: Some(first.clone()),
    };
    // Only now the parent: the thread, as exactly one indexed user session.
    let parent = match store
        .user_sessions_with_native(Host::Codex, &candidate.key.parent_native_session_id)?
        .as_slice()
    {
        [one] => Some(one.clone()),
        _ => None,
    };
    let proof = parent.map(|parent| ClaudeLaunchCreationProof {
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
    });
    Ok(Resolve::Ready(Box::new(Ready {
        fact,
        proof,
        row,
        members,
        transcript,
        transcript_generation: generation,
    })))
}

/// What the recorded source lines say, including a one-use binding when the
/// candidate cites its two witness lines.
fn recorded(
    row: &CandidateRow,
    launch: &[u8],
    acknowledgment: &[u8],
    binding_call: Option<&[u8]>,
    binding_output: Option<&[u8]>,
    sources: &Sources<'_>,
    allowance: &Allowance,
) -> Result<Recorded, Outcome> {
    let candidate = &row.candidate;
    let rejected = Outcome::SourceRejected;
    let _decoding = allowance
        .reserve(
            source::decoded_bound(launch)
                .saturating_add(rows::row_bound(launch))
                .saturating_add(rows::row_bound(acknowledgment))
                .saturating_add(binding_call.map_or(0, |line| {
                    source::decoded_bound(line) + rows::row_bound(line)
                }))
                .saturating_add(binding_output.map_or(0, |line| {
                    source::decoded_bound(line) + rows::row_bound(line)
                })),
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
    let ops = match (binding_call, binding_output) {
        (None, None) => cell::parse_operations(&input).map_err(|()| rejected)?,
        (Some(call_line), Some(output_line)) => {
            let call_head = rows::row(call_line).ok_or(rejected)?;
            let call_value: Value = serde_json::from_slice(call_line).map_err(|_| rejected)?;
            let Some(rows::Item::Call {
                call_id: binding_id,
                input: binding_code,
                ..
            }) = rows::item(&call_value).map_err(|()| rejected)?
            else {
                return Err(rejected);
            };
            if !call_head.is_exec() {
                return Err(rejected);
            }
            let binding = cell::parse_binding(&binding_code).map_err(|()| rejected)?;
            let output_head = rows::row(output_line).ok_or(rejected)?;
            let output_value: Value = serde_json::from_slice(output_line).map_err(|_| rejected)?;
            let Some(rows::Item::Output { call_id, items }) =
                rows::item(&output_value).map_err(|()| rejected)?
            else {
                return Err(rejected);
            };
            if !output_head.is_output() || call_id != binding_id {
                return Err(rejected);
            }
            let id = rows::binding_receipt(
                &items,
                binding.prefix,
                binding.operations,
                binding.literal.as_deref(),
            )
            .ok_or(rejected)?;
            cell::parse_loaded(&input, &binding.key, &id).map_err(|()| rejected)?
        }
        _ => return Err(rejected),
    };
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
        .reserve(submitted.prompt.as_ref().map_or(0, String::capacity))
        .map_err(|_| Outcome::Deferred)?;
    Ok(Recorded {
        prompt: submitted.prompt,
        launched_ms,
        _prompt: reserved,
    })
}

/// The time of a `codex exec --json` launch and its cell's operation count,
/// when its recorded lines still read as that launch: the launch row is this
/// candidate's own `exec` call, whose operation is a fresh
/// `codex exec --json` command in a cell that loads no binding, and its
/// acknowledgment row is this candidate's own output row. Which result in it
/// is the launch's own, and the thread it names, are found again by replaying
/// the launch's follower ([`codex_replay`]).
fn recorded_codex(
    row: &CandidateRow,
    launch: &[u8],
    acknowledgment: &[u8],
    bound: bool,
    allowance: &Allowance,
) -> Result<(i64, usize), Outcome> {
    let candidate = &row.candidate;
    let rejected = Outcome::SourceRejected;
    if bound {
        return Err(rejected);
    }
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
    let op = candidate.key.launch_operation_index as usize;
    ops.get(op)
        .filter(|operation| operation.tool == Tool::ExecCommand)
        .and_then(|operation| operation.str("cmd"))
        .filter(|cmd| codex_cli::launch(cmd))
        .ok_or(rejected)?;
    let launched_ms = rows::timestamp(&value).ok_or(rejected)?;
    let answered = rows::row(acknowledgment).ok_or(rejected)?;
    if !answered.is_output()
        || answered.call_id() != Some(candidate.acknowledgment_call_id.as_str())
        || answered.ordinal != candidate.acknowledgment_ordinal
    {
        return Err(rejected);
    }
    Ok((launched_ms, ops.len()))
}

/// One row of a replayed launch window, fed to the launch's follower as the
/// scan fed it: a row that breaks the history's structure, or an output
/// that pairs with no call, ends the replay; a tool call or output is
/// decoded whole; the row holding the follower's acknowledgment is noted.
fn replay_row(
    offset: u64,
    bytes: &[u8],
    replay: &mut ReplayState,
    allowance: &Allowance,
) -> Result<(), Unread> {
    if replay.broken || replay.acknowledged.is_some() {
        replay.broken = true;
        return Ok(());
    }
    let _head = allowance.reserve(rows::row_bound(bytes))?;
    let Some(row) = rows::row(bytes) else {
        replay.broken = true;
        return Ok(());
    };
    if row.kind.as_deref() == Some("session_meta") || row.unpairable_output() {
        replay.broken = true;
        return Ok(());
    }
    if !row.is_call() && !row.is_output() {
        return Ok(());
    }
    let _decoding = allowance.reserve(source::decoded_bound(bytes))?;
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        replay.broken = true;
        return Ok(());
    };
    let Ok(Some(item)) = rows::item(&value) else {
        replay.broken = true;
        return Ok(());
    };
    match replay.ack.feed(&item) {
        Ok(None) => {}
        Ok(Some(acknowledged)) => replay.acknowledged = Some((offset, acknowledged)),
        Err(Broken::Memory) => return Err(Unread::Memory),
        Err(_) => replay.broken = true,
    }
    Ok(())
}

/// Continue replaying a Codex launch's follower over the parent's rows after
/// the launch row up to and including its acknowledgment row, within the
/// pass's budget, as the published generation. The follower must
/// acknowledge exactly at that row, by the same call, with the same process
/// handle and the same thread: so the result checked is the launch's own,
/// at its own position among what its cell emitted, whichever output held
/// it. Then the child's header is read.
fn codex_replay(
    store: &Store,
    mut resolution: Box<Resolution>,
    sources: &Sources<'_>,
    allowance: &Allowance,
    budget: &mut Budget<'_>,
) -> xt_store::Result<Resolve> {
    let finished = |outcome| Ok(Resolve::Finished(outcome));
    let Phase::Replay {
        file,
        lines,
        state,
        end,
        launched_ms,
    } = &mut resolution.phase
    else {
        unreachable!("a Codex launch is replayed first");
    };
    while lines.next < *end && !state.broken {
        let chunk = match source::read_chunk(file, lines.next, *end - lines.next, budget) {
            Ok(Some(chunk)) => chunk,
            Ok(None) => return Ok(Resolve::Paused(resolution)),
            Err(_) => return finished(Outcome::Stale),
        };
        let state = &mut **state;
        match lines.feed(&chunk, &mut |offset, bytes| {
            replay_row(offset, bytes, state, allowance)
        }) {
            Ok(()) => {}
            Err(Unread::Memory) => return finished(Outcome::Deferred),
            Err(_) => return finished(Outcome::SourceRejected),
        }
    }
    if source::generation_of(file).ok() != Some(resolution.generation)
        || source::stat(&resolution.segment).ok() != Some(resolution.generation)
    {
        return finished(Outcome::Stale);
    }
    let candidate = &resolution.row.candidate;
    let acknowledged = std::mem::take(&mut state.acknowledged);
    let own = !state.broken
        && lines.finished()
        && acknowledged.is_some_and(|(at, acknowledged)| {
            u64::try_from(candidate.acknowledgment_offset).ok() == Some(at)
                && acknowledged.call == candidate.acknowledgment_call_id
                && acknowledged.handle == candidate.process_session_id
                && acknowledged.thread.as_deref()
                    == Some(candidate.child_native_session_id.as_str())
        });
    if !own {
        return finished(Outcome::SourceRejected);
    }
    let launched_ms = *launched_ms;
    let (rollout, opened, generation, read) =
        match codex_header(store, &resolution.row, sources, allowance)? {
            Ok(header) => header,
            Err(outcome) => return finished(outcome),
        };
    if resolution.held.grow(rollout.as_os_str().len()).is_err() {
        return finished(Outcome::Deferred);
    }
    resolution.phase = Phase::Header {
        rollout,
        file: opened,
        generation,
        read: Box::new(read),
        launched_ms,
    };
    codex_child(store, resolution, allowance, budget)
}

/// A Codex child's one recorded original rollout, contained in the Codex
/// history root and opened as one regular file, ready for its opening line
/// to be read; or what became of the check.
#[allow(clippy::type_complexity)]
fn codex_header(
    store: &Store,
    row: &CandidateRow,
    sources: &Sources<'_>,
    allowance: &Allowance,
) -> xt_store::Result<Result<(PathBuf, File, SegmentGeneration, LineRead), Outcome>> {
    let child = &row.candidate.child_native_session_id;
    let originals: Vec<PathBuf> = group::indexed_paths(store, child)?
        .into_iter()
        .filter(|path| codex_segment(path, child) == Some(Segment::Root))
        .collect();
    let [rollout] = originals.as_slice() else {
        return Ok(Err(Outcome::Unclear));
    };
    let mut roots = host_roots(Host::Codex, sources.home);
    for root in roots.clone() {
        if let Ok(resolved) = root.canonicalize()
            && !roots.contains(&resolved)
        {
            roots.push(resolved);
        }
    }
    if !codex_contained(rollout, &roots, child) {
        return Ok(Err(Outcome::Unclear));
    }
    let (file, generation) = match source::open(rollout) {
        Ok(opened) => opened,
        Err(Unread::Alias) => return Ok(Err(Outcome::Unclear)),
        Err(_) => return Ok(Err(Outcome::Retry)),
    };
    let Ok(read) = LineRead::new(0, sources.max_line, allowance) else {
        return Ok(Err(Outcome::Deferred));
    };
    Ok(Ok((rollout.clone(), file, generation, read)))
}

/// Whether a Codex rollout's opening line is the fresh header of the thread
/// `child` that a `codex exec` run opened after `launched_ms`
/// ([`fresh_exec_opened`]).
fn fresh_header(line: &[u8], child: &str, launched_ms: i64) -> bool {
    fresh_exec_opened(line, child).is_some_and(|opened| opened > launched_ms)
}

/// When the Codex thread `child` was opened, if its rollout's opening line
/// is the fresh header a `codex exec` run writes: this thread's own
/// `session_meta` ([`group::header`]), a person's own role with no history
/// base, fork, copied history or inherited rows, `source` exactly `exec`, no
/// parent or fork origin named, and its own valid `timestamp`. The one
/// applicability the launch resolution and the display check share.
pub(in crate::native) fn fresh_exec_opened(line: &[u8], child: &str) -> Option<i64> {
    let header = group::header(line, child).ok()?;
    if header.role != Role::Own
        || header.base.is_some()
        || header.forked
        || header.copied
        || header.inherited_below.is_some()
    {
        return None;
    }
    let value = serde_json::from_slice::<Value>(line).ok()?;
    let payload = &value["payload"];
    (payload.get("source").and_then(Value::as_str) == Some("exec")
        && ["parent_thread_id", "forked_from_id"]
            .iter()
            .all(|key| payload.get(key).is_none_or(Value::is_null)))
    .then_some(())?;
    payload
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(rows::millis)
}

/// Continue a Codex child's check: its opening line, still the generation
/// it was opened at, must be a fresh `exec` header, and the index must hold
/// its first eligible input, dated at or after the launch. Then the fact and,
/// with exactly one indexed parent, the relation.
fn codex_child(
    store: &Store,
    mut resolution: Box<Resolution>,
    allowance: &Allowance,
    budget: &mut Budget<'_>,
) -> xt_store::Result<Resolve> {
    let finished = |outcome| Ok(Resolve::Finished(outcome));
    let Phase::Header {
        rollout,
        file,
        generation,
        read,
        launched_ms,
    } = &mut resolution.phase
    else {
        unreachable!("a Codex child is read from its header");
    };
    let line = match read.advance(file, budget) {
        Ok(Some(line)) => line,
        Ok(None) => return Ok(Resolve::Paused(resolution)),
        Err(Unread::Memory) => return finished(Outcome::Deferred),
        Err(Unread::TooLarge | Unread::Alias) => return finished(Outcome::Unclear),
        Err(_) => return finished(Outcome::Retry),
    };
    let (generation, launched_ms) = (*generation, *launched_ms);
    if source::generation_of(file).ok() != Some(generation)
        || source::stat(rollout).ok() != Some(generation)
    {
        return finished(Outcome::Retry);
    }
    let candidate = &resolution.row.candidate;
    let fresh = {
        let Ok(_decoding) =
            allowance.reserve(source::decoded_bound(&line) + rows::row_bound(&line))
        else {
            return finished(Outcome::Deferred);
        };
        fresh_header(&line, &candidate.child_native_session_id, launched_ms)
    };
    drop(line);
    if !fresh {
        return finished(Outcome::Rejected);
    }
    let Some((first, first_ms)) = store.first_input_record(&resolution.child)? else {
        return finished(Outcome::Waiting);
    };
    if first_ms < launched_ms {
        return finished(Outcome::Rejected);
    }
    let rollout = std::mem::take(rollout);
    let Resolution {
        row,
        members,
        child,
        ..
    } = *resolution;
    let candidate = &row.candidate;
    let fact = ChildFact {
        child_session_id: child.clone(),
        child_host: Host::Codex,
        child_native_session_id: candidate.child_native_session_id.clone(),
        evidence_kind: ChildEvidence::CodexCliLaunch,
        evidence_version: CODEX_CLI_LAUNCH_VERSION,
        source_native_session_id: candidate.key.parent_native_session_id.clone(),
        source_rollout_id: Some(candidate.key.rollout_id.clone()),
        launch_call_id: Some(candidate.key.launch_call_id.clone()),
        launch_operation_index: Some(candidate.key.launch_operation_index),
        first_record_uuid: Some(first.clone()),
    };
    // Only now the parent: the thread, as exactly one indexed user session.
    let parent = match store
        .user_sessions_with_native(Host::Codex, &candidate.key.parent_native_session_id)?
        .as_slice()
    {
        [one] => Some(one.clone()),
        _ => None,
    };
    let proof = parent.map(|parent| ClaudeLaunchCreationProof {
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
        evidence_version: CODEX_CLI_LAUNCH_VERSION,
    });
    Ok(Resolve::Ready(Box::new(Ready {
        fact,
        proof,
        row,
        members,
        transcript: rollout,
        transcript_generation: generation,
    })))
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
pub(super) struct ChildStream {
    file: File,
    path: PathBuf,
    pub generation: SegmentGeneration,
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
    /// What broke it was a bound or a line that is not well formed, not a
    /// line that disagrees with the launch.
    unclear: bool,
    /// The first turn, when the check asked for it.
    pub turn: Option<Box<Turn>>,
}

/// Why a child transcript is not the launch's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refused {
    /// Its lines were read and disagree with the launch.
    Mismatch,
    /// It is beyond this version's bounds, or not well formed: nothing was
    /// decided.
    Unclear,
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
            + self.recorded.prompt.as_ref().map_or(0, String::capacity)
            + source::table_actual(self.before_first.capacity(), std::mem::size_of::<String>())
            + self
                .before_first
                .iter()
                .map(String::capacity)
                .sum::<usize>()
            + self.first.as_ref().map_or(0, |first| first.uuid.capacity())
    }

    pub(super) fn open(
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
            unclear: false,
            turn: None,
        })
    }

    /// Read on within `budget`: `None` when paused, else the first input's
    /// identifier or why the transcript is not the launch's.
    pub(super) fn advance(
        &mut self,
        budget: &mut Budget<'_>,
    ) -> Result<Option<Result<String, Refused>>, Unread> {
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
                unclear,
                turn,
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
                unclear,
                turn: turn.as_deref_mut(),
            };
            lines.feed(&chunk, &mut |_, bytes| {
                if bytes.is_empty() {
                    return Ok(());
                }
                let _decoding = allowance.reserve(source::decoded_bound(bytes))?;
                state.line(bytes)
            })?;
        }
        let after = source::generation_of(&self.file)?;
        if after != self.generation || source::stat(&self.path)? != self.generation {
            return Err(Unread::Changed);
        }
        if self.broken {
            return Ok(Some(Err(if self.unclear {
                Refused::Unclear
            } else {
                Refused::Mismatch
            })));
        }
        // A last line still being written: the prefix decides nothing.
        if !self.lines.finished() {
            return Err(Unread::Unfinished);
        }
        Ok(Some(match &self.first {
            Some(first) if first.matches && first.named == 1 => Ok(first.uuid.clone()),
            _ => Err(Refused::Mismatch),
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
    unclear: &'a mut bool,
    turn: Option<&'a mut Turn>,
}

impl LineState<'_> {
    /// Broken by a bound or a line that is not well formed.
    fn unclear(&mut self) -> Result<(), Unread> {
        *self.broken = true;
        *self.unclear = true;
        Ok(())
    }

    fn line(&mut self, line: &[u8]) -> Result<(), Unread> {
        if *self.broken {
            return Ok(());
        }
        *self.count += 1;
        if *self.count > self.max_lines {
            return self.unclear();
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            return self.unclear();
        };
        if !value.is_object() {
            return self.unclear();
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
                None => return self.unclear(),
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
                if let Some(turn) = self.turn.as_deref_mut() {
                    turn.line(&value, at, input);
                }
            }
            None if input => {
                let (Some(uuid), Some(at)) = (uuid, at) else {
                    return self.unclear();
                };
                self.before_reserved.grow(uuid.len())?;
                // A literal prompt is compared; a redirected one, never
                // opened, only asks for one text.
                let text = text.flatten();
                *self.first = Some(First {
                    uuid: uuid.to_owned(),
                    ts_ms: at,
                    matches: match &self.recorded.prompt {
                        Some(prompt) => text == Some(prompt.as_str()),
                        None => text.is_some(),
                    },
                    named: 1 + usize::from(self.before_first.contains(uuid)),
                });
                if let Some(turn) = self.turn.as_deref_mut() {
                    turn.first(&value, Some(at));
                }
            }
            None => {
                if let Some(uuid) = uuid
                    && !self.before_first.contains(uuid)
                {
                    if self.before_first.len() >= MAX_BEFORE_FIRST {
                        return self.unclear();
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
pub(super) enum Walk {
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
pub(super) fn sole_transcript(
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
    fn check(lines: &[String]) -> Result<String, Refused> {
        check_with(lines, Some(PROMPT))
    }

    /// [`check`] for a launch with this literal prompt, or a redirected one.
    fn check_with(lines: &[String], prompt: Option<&str>) -> Result<String, Refused> {
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
            max_window_bytes: 1 << 20,
            unfinished: &unfinished,
        };
        let allowance = Allowance::new(1 << 20);
        let mut stream = ChildStream::open(
            &path,
            CHILD,
            Recorded {
                prompt: prompt.map(str::to_owned),
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

    #[test]
    fn a_replaced_child_source_cannot_finish_a_negative_decision() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join(format!("{CHILD}.jsonl"));
        std::fs::write(
            &path,
            json!({"type":"x","sessionId":"another-session"}).to_string() + "\n",
        )
        .unwrap();
        let unfinished = HashMap::new();
        let sources = Sources {
            home: temp.path(),
            programs: &[],
            max_line: 1 << 20,
            max_child_bytes: 1 << 20,
            max_child_lines: 1000,
            max_window_bytes: 1 << 20,
            unfinished: &unfinished,
        };
        let allowance = Allowance::new(1 << 20);
        let mut stream = ChildStream::open(
            &path,
            CHILD,
            Recorded {
                prompt: Some(PROMPT.into()),
                launched_ms: rows::millis("2026-09-29T07:58:36.333Z").unwrap(),
                _prompt: allowance.reserve(PROMPT.len()).unwrap(),
            },
            &sources,
            &allowance,
        )
        .unwrap();
        let old = temp.path().join("old.jsonl");
        std::fs::rename(&path, &old).unwrap();
        std::fs::write(
            &path,
            user("f", "2026-09-29T07:58:40.536Z", json!(PROMPT)) + "\n",
        )
        .unwrap();
        let mut budget = Budget::new(1 << 20, Duration::from_secs(60), None);
        assert!(matches!(stream.advance(&mut budget), Err(Unread::Changed)));
    }

    /// A `codex exec --json` launch's recorded rows are read again: the
    /// launch row must still be this call running the fresh command in a
    /// cell that loads no binding, and the acknowledgment row this call's
    /// output. Which of its results is the launch's is the replay's.
    #[test]
    fn a_codex_launch_row_is_read_again_from_its_own_line() {
        let fresh = "codex exec --json -c x=1 'Synthetic'";
        let own = codex_row("call_launch", 0);
        let at = rows::millis("2026-10-05T22:17:23.642Z").unwrap();
        let allowance = Allowance::new(1 << 20);
        let read = |row: &CandidateRow, launch: &[u8], answer: &[u8], bound: bool| {
            recorded_codex(row, launch, answer, bound, &allowance)
        };
        let answer = output_row("call_launch", 3, &[started(THREAD)]);
        assert_eq!(
            read(&own, &exec_row("call_launch", 2, &[fresh]), &answer, false),
            Ok((at, 1))
        );
        assert_eq!(
            read(
                &own,
                &exec_row("call_launch", 2, &["git status", fresh]),
                &answer,
                false
            ),
            Err(Outcome::SourceRejected),
            "operation 0 is not the launch"
        );
        for (launch, answer, bound) in [
            (exec_row("call_launch", 2, &[fresh]), answer.clone(), true),
            (
                exec_row(
                    "call_launch",
                    2,
                    &[&format!("codex exec --json resume {THREAD}")],
                ),
                answer.clone(),
                false,
            ),
            (
                exec_row("call_launch", 2, &["codex exec 'x'"]),
                answer.clone(),
                false,
            ),
            (exec_row("call_other", 2, &[fresh]), answer.clone(), false),
            (exec_row("call_launch", 7, &[fresh]), answer.clone(), false),
            (
                exec_row("call_launch", 2, &[fresh]),
                output_row("call_other", 3, &[started(THREAD)]),
                false,
            ),
            (
                exec_row("call_launch", 2, &[fresh]),
                output_row("call_launch", 9, &[started(THREAD)]),
                false,
            ),
        ] {
            assert_eq!(
                read(&own, &launch, &answer, bound),
                Err(Outcome::SourceRejected)
            );
        }
    }

    const PARENT: &str = "01a00000-0000-7000-8000-0000000000aa";
    const THREAD: &str = "01a10000-0000-7000-8000-0000000000c1";
    const OTHER: &str = "01a10000-0000-7000-8000-0000000000c2";

    fn exec_row(call: &str, ordinal: i64, commands: &[&str]) -> Vec<u8> {
        let input: String = commands
            .iter()
            .map(|command| {
                format!(
                    "text(await tools.exec_command({{cmd: {}, yield_time_ms: 1000}}));\n",
                    serde_json::to_string(command).unwrap()
                )
            })
            .collect();
        json!({"timestamp": "2026-10-05T22:17:23.642Z", "ordinal": ordinal,
            "type": "response_item", "payload": {"type": "custom_tool_call", "call_id": call,
            "name": "exec", "input": input}})
        .to_string()
        .into_bytes()
    }

    fn started(thread: &str) -> String {
        json!({"session_id": 69773, "output": format!(
            "Reading additional input from stdin...\n{{\"type\":\"thread.started\",\"thread_id\":\"{thread}\"}}\n")})
        .to_string()
    }

    fn cell_output(call: &str, ordinal: i64, header: &str, results: &[String]) -> Vec<u8> {
        let mut items = vec![json!({"type": "input_text", "text": header})];
        items.extend(
            results
                .iter()
                .map(|text| json!({"type": "input_text", "text": text})),
        );
        json!({"timestamp": "2026-10-05T22:17:30.961Z", "ordinal": ordinal,
            "type": "response_item",
            "payload": {"type": "custom_tool_call_output", "call_id": call, "output": items}})
        .to_string()
        .into_bytes()
    }

    fn output_row(call: &str, ordinal: i64, results: &[String]) -> Vec<u8> {
        cell_output(call, ordinal, "Script completed\nOutput:\n", results)
    }

    fn codex_row(acknowledged: &str, op: u32) -> CandidateRow {
        use xt_store::claude_launch::{LaunchCandidate, LaunchKey, SourceVerdict};
        CandidateRow {
            candidate: LaunchCandidate {
                key: LaunchKey {
                    parent_native_session_id: PARENT.into(),
                    rollout_id: PARENT.into(),
                    launch_call_id: "call_launch".into(),
                    launch_operation_index: op,
                },
                child_native_session_id: THREAD.into(),
                acknowledgment_call_id: acknowledged.into(),
                acknowledgment_operation_index: op,
                process_session_id: Some("69773".into()),
                launch_offset: 10,
                acknowledgment_offset: 20,
                launch_ordinal: Some(2),
                acknowledgment_ordinal: Some(3),
                binding_call_offset: None,
                binding_output_offset: None,
                child_host: Host::Codex,
                launch_check_fingerprint: Some("0".repeat(64)),
            },
            source_revision: 1,
            source_verdict: SourceVerdict::Valid,
            child_state: ChildState::Waiting,
        }
    }

    /// Replay the follower of operation `op` of `ops` over `rows`, the rows
    /// after the launch row: where it acknowledged, by which call, naming
    /// which thread, or `None` when it did not or a row ended the replay.
    fn replay(op: usize, ops: usize, rows: &[Vec<u8>]) -> Option<(u64, String, Option<String>)> {
        let allowance = Allowance::new(1 << 20);
        let mut state = ReplayState {
            ack: Ack::new("call_launch", op, ops, &allowance)
                .unwrap()
                .reading_thread(),
            acknowledged: None,
            broken: false,
        };
        for (offset, row) in rows.iter().enumerate() {
            replay_row(offset as u64, row, &mut state, &allowance).unwrap();
        }
        if state.broken {
            return None;
        }
        state
            .acknowledged
            .map(|(at, acknowledged)| (at, acknowledged.call, acknowledged.thread))
    }

    fn wait_row(call: &str, cell: &str) -> Vec<u8> {
        json!({"timestamp": "2026-10-05T22:17:24Z", "type": "response_item", "payload": {
            "type": "function_call", "call_id": call, "name": "wait",
            "arguments": json!({"cell_id": cell, "yield_time_ms": 30000}).to_string()}})
        .to_string()
        .into_bytes()
    }

    /// Each operation's own result, wherever its cell emitted it: two
    /// launches both answered in one wait, or one in the launch's own output
    /// and the other in the wait, each names only its own thread. Another
    /// call's output names neither.
    #[test]
    fn a_replayed_follower_takes_each_operations_own_result() {
        let running = "Script running with cell ID 12\n";
        let both_in_wait = [
            cell_output("call_launch", 3, running, &[]),
            wait_row("call_wait", "12"),
            output_row("call_other", 5, &[started(OTHER)]),
            output_row("call_wait", 6, &[started(THREAD), started(OTHER)]),
        ];
        assert_eq!(
            replay(0, 2, &both_in_wait),
            Some((3, "call_wait".into(), Some(THREAD.into())))
        );
        assert_eq!(
            replay(1, 2, &both_in_wait),
            Some((3, "call_wait".into(), Some(OTHER.into())))
        );
        let first_already = [
            cell_output("call_launch", 3, running, &[started(THREAD)]),
            wait_row("call_wait", "12"),
            output_row("call_wait", 5, &[started(OTHER)]),
        ];
        // The window ends at the launch's own acknowledgment row; a row after
        // an acknowledgment means the follower answered elsewhere.
        assert_eq!(
            replay(0, 2, &first_already[..1]),
            Some((0, "call_launch".into(), Some(THREAD.into())))
        );
        assert_eq!(replay(0, 2, &first_already), None);
        assert_eq!(
            replay(1, 2, &first_already),
            Some((2, "call_wait".into(), Some(OTHER.into())))
        );
        // A completed wait must hold one result per remaining operation.
        assert_eq!(
            replay(
                1,
                2,
                &[
                    cell_output("call_launch", 3, running, &[]),
                    wait_row("call_wait", "12"),
                    output_row("call_wait", 5, &[started(OTHER)]),
                ]
            ),
            None
        );
        // Only another call's output: no acknowledgment.
        assert_eq!(
            replay(0, 1, &[output_row("call_other", 3, &[started(THREAD)])]),
            None
        );
        // An output that pairs with no call ends the replay.
        let unpairable = json!({"type": "response_item", "payload": {
            "type": "custom_tool_call_output", "id": "ctco_x", "output": "x"}})
        .to_string()
        .into_bytes();
        assert_eq!(
            replay(
                0,
                1,
                &[unpairable, output_row("call_launch", 4, &[started(THREAD)])]
            ),
            None
        );
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
                Err(Refused::Mismatch),
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
            assert_eq!(check(&lines), Err(Refused::Mismatch), "{lines:?}");
        }
    }

    /// A transcript beyond the bounds, or with a line that is not well
    /// formed, decides nothing: the launch's child check stays open, apart
    /// from one whose lines disagree with the launch.
    #[test]
    fn a_transcript_that_cannot_be_read_as_one_is_unclear_not_a_mismatch() {
        let first = user("f", "2026-09-29T07:58:40.536Z", json!(PROMPT));
        for lines in [
            vec![first.clone(), "{not json".to_owned()],
            vec![first.clone(), "[1,2]".to_owned()],
            vec![
                first.clone(),
                json!({"type": "x", "timestamp": "yesterday", "sessionId": CHILD}).to_string(),
            ],
            vec![
                json!({"type": "user", "sessionId": CHILD,
                "message": {"role": "user", "content": PROMPT}})
                .to_string(),
            ],
            std::iter::repeat_n(json!({"type": "x", "sessionId": CHILD}).to_string(), 1001)
                .collect(),
        ] {
            assert_eq!(check(&lines), Err(Refused::Unclear), "{:?}", &lines[..1]);
        }
    }

    /// A prompt read from a redirected file is never opened: the fresh
    /// child's first input needs only to be one text, after the launch,
    /// unique as the first and on every line its own; it may come long after
    /// the launch's acknowledgment. Everything else still refuses it.
    #[test]
    fn a_redirected_prompt_binds_the_fresh_child_without_comparing_text() {
        let first = user("f", "2026-09-29T07:58:40.536Z", json!("Any brief at all"));
        assert_eq!(
            check_with(std::slice::from_ref(&first), None),
            Ok("f".into())
        );
        assert_eq!(
            check_with(
                &[user(
                    "f",
                    "2026-10-09T08:20:00Z",
                    json!([{"type": "text", "text": "x"}])
                )],
                None
            ),
            Ok("f".into())
        );
        for lines in [
            // An image first, a text after it.
            vec![
                user("i", "2026-09-29T07:58:39Z", json!([{"type": "image"}])),
                first.clone(),
            ],
            // Before the launch: an older session reusing the identifier.
            vec![user("f", "2026-09-29T07:58:30Z", json!("Reused"))],
            vec![
                json!({"type": "summary", "timestamp": "2026-09-28T10:00:00Z",
                    "sessionId": CHILD})
                .to_string(),
                first.clone(),
            ],
            // Another session's line, the record twice, a competing input.
            vec![
                first.clone(),
                json!({"type": "x", "sessionId": "other"}).to_string(),
            ],
            vec![first.clone(), first.clone()],
            vec![
                first.clone(),
                user("e", "2026-09-29T07:58:40.536Z", json!("Same time")),
            ],
        ] {
            assert_eq!(
                check_with(&lines, None),
                Err(Refused::Mismatch),
                "{lines:?}"
            );
        }
    }
}
