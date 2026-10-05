//! One read of one physical history file of a thread, from its first byte,
//! spread over as many passes as its size and each pass's budget need.
//!
//! Every complete row is decoded for its structure: its type (no second
//! `session_meta`), its actual ordinal (consecutive from the header's where
//! the file carries ordinals, absent where it does not) and the call
//! identifier of every tool call and output. Every occurrence of a call
//! identifier is counted — calls and outputs apart, each saturating at "more
//! than one" — under a keyed hash shared by the whole run, so the thread's
//! validation can check, across all its files, that each identifier a launch
//! uses occurs exactly once. Only `exec` calls and rows inside an open
//! launch's bounded window are decoded whole. A launch operation
//! ([`super::command`]) in one of the thread's own rows is followed
//! ([`super::ack`]) by exact identifiers to its own first process result, and
//! no further; one not acknowledged by the end is not a launch of this
//! generation.
//!
//! A running cell belongs to the exec call whose output announced it, until
//! a `wait` on it sees it complete: every own row's output header is read for
//! it, so a cell two calls claim at once is known whether or not a launch was
//! open then, and every launch waiting on such a cell is broken. A process
//! handle is not an identity: what becomes of it, or of its number, after a
//! launch's acknowledgment is never read.
//!
//! What the read has established — its place, the partial line, the ordinal,
//! the counts, the running cells, the open launches, the launches found —
//! stays in memory between passes, reserved from the thread's allowance at
//! actual capacity before it grows. Nothing of it is stored: a restart reads the
//! file again. The file is read only as the generation its header was
//! planned from, and when the read ends it must still be exactly that
//! generation, else nothing it found is used. A file whose last line has no
//! newline yet is unfinished: nothing of it is used until it is complete.

use super::ack::{Ack, Acknowledged, Broken};
use super::cell::{self, Tool, Value as Arg};
use super::command;
use super::group::Segment;
use super::rows::{self, CellHeader, Item};
use super::source::{self, Allowance, Budget, Lines, Reserved, Unread};
use serde_json::Value;
use std::{
    collections::{HashMap, hash_map::RandomState},
    fs::File,
    hash::BuildHasher,
    path::PathBuf,
};
use xt_store::claude_launch::SegmentGeneration;

/// The ceilings one member read observes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ScanBounds {
    pub max_line: usize,
    pub max_window_bytes: u64,
    pub max_window_lines: usize,
    /// Launches followed at once; a later one beyond it is not followed.
    pub max_open: usize,
}

/// Saturating occurrence counts of one call identifier: 0, 1, or 2 for
/// "more than one".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Counts {
    pub calls: u8,
    pub outputs: u8,
}

impl Counts {
    pub fn plus(self, other: Self) -> Self {
        Self {
            calls: (self.calls + other.calls).min(2),
            outputs: (self.outputs + other.outputs).min(2),
        }
    }
}

/// The hash of one call identifier under the run's key.
pub(super) fn id_hash(key: &RandomState, id: &str) -> u64 {
    key.hash_one(id)
}

/// A launch acknowledged in this file by its own first process result, in
/// the output of `acknowledgment_call_id` (the launch call, or a `wait` on
/// its cell) at `acknowledgment_offset`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Found {
    pub launch_call_id: String,
    pub launch_op: u32,
    pub child: String,
    pub acknowledgment_call_id: String,
    /// The process handle, when the result said the process was running.
    pub handle: Option<String>,
    pub launch_offset: u64,
    pub acknowledgment_offset: u64,
    pub launch_ordinal: Option<i64>,
    pub acknowledgment_ordinal: Option<i64>,
    /// The hashes of every call identifier followed: the launch and each
    /// wait on its cell.
    pub ids: Vec<u64>,
}

impl Found {
    /// Bytes its strings and identifier hashes hold, at capacity.
    pub fn bytes(&self) -> usize {
        self.launch_call_id.capacity()
            + self.child.capacity()
            + self.acknowledgment_call_id.capacity()
            + self.handle.as_ref().map_or(0, String::capacity)
            + self.ids.capacity() * 8
    }
}

/// What one member read established.
#[derive(Debug)]
pub(super) struct MemberFacts {
    pub rollout: String,
    /// The generation read, from the open descriptor.
    pub generation: SegmentGeneration,
    /// The end of the last complete line read.
    pub complete: u64,
    /// `false` when a row broke the file's structure, or was longer than a
    /// row may be: nothing in the file is used and the thread is invalid.
    pub valid: bool,
    /// Every call identifier's occurrence counts, by hash.
    pub counts: HashMap<u64, Counts>,
    pub chains: Vec<Found>,
    /// Launches that ended with no acknowledgment, or ran too long.
    pub broken: usize,
    /// The map entries and bytes these facts hold, reserved.
    pub entries: Reserved,
    pub bytes: Reserved,
}

impl MemberFacts {
    /// Bytes the facts hold apart from their call-count map (bounded by
    /// entries), at capacity: themselves boxed, their rollout and launches.
    pub fn held(rollout: &String, chains: &Vec<Found>) -> usize {
        std::mem::size_of::<Self>()
            + rollout.capacity()
            + chains.capacity() * std::mem::size_of::<Found>()
            + chains.iter().map(Found::bytes).sum::<usize>()
    }
}

/// Where a read is after one pass.
#[derive(Debug)]
pub(super) enum Step {
    /// The pass's budget ended; the read resumes here next pass.
    Paused,
    Done(Box<MemberFacts>),
    /// The file's last line has no newline yet: nothing of this generation
    /// is used.
    Unfinished,
}

/// Why a read ended with nothing usable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScanError {
    /// The file is not the one being read any more, or could not be read
    /// now: the read is dropped and tried again later.
    Source(Unread),
    /// The thread's own history needs more memory or map entries than its
    /// allowance: it is recorded invalid.
    OverLimit,
}

/// `exec_command` options a launch may carry, and their kinds.
fn launch_args_allowed(op: &cell::Cell) -> bool {
    op.args.iter().all(|(key, value)| match key.as_str() {
        "cmd" | "workdir" | "sandbox_permissions" | "justification" => {
            matches!(value, Arg::Str(_))
        }
        "yield_time_ms" | "max_output_tokens" => matches!(value, Arg::Num(_)),
        "tty" => matches!(value, Arg::Bool(false)),
        _ => false,
    })
}

/// A call identifier the tables can hold: 1–256 printable ASCII characters.
pub(super) fn call_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

#[derive(Debug)]
struct Open {
    ack: Ack,
    launch_call_id: String,
    launch_op: u32,
    child: String,
    launch_offset: u64,
    launch_ordinal: Option<i64>,
    window_bytes: u64,
    window_lines: usize,
    /// Its two strings.
    _held: Reserved,
}

/// The running cells of a file's own rows — each with the exec call that
/// announced it, or `None` once two calls claimed it — and the `wait` calls
/// not yet answered, with the cell each waits on.
#[derive(Debug)]
struct Cells {
    running: HashMap<String, Option<String>>,
    waits: HashMap<String, String>,
    /// Bytes the keys and values hold.
    strings: usize,
    held: Reserved,
    entries: Reserved,
}

const PAIR: usize = 2 * std::mem::size_of::<String>();

impl Cells {
    fn bytes(&self) -> usize {
        source::table_bytes(self.running.capacity(), PAIR)
            + source::table_bytes(self.waits.capacity(), PAIR)
            + self.strings
    }

    /// Reserve an entry of `strings` bytes in a map of `len` entries and
    /// `capacity`, and the map's growth, before it is inserted.
    fn room(&mut self, len: usize, capacity: usize, strings: usize) -> Result<(), Unread> {
        let growth = if len == capacity {
            source::table_bytes(capacity.saturating_mul(2).max(8), PAIR)
        } else {
            0
        };
        self.held.resize(self.bytes() + growth + strings)?;
        self.entries.grow(1)
    }

    fn fit(&mut self) -> Result<(), Unread> {
        self.held.resize(self.bytes())?;
        self.entries.resize(self.running.len() + self.waits.len())
    }

    /// `call` announced `cell` running.
    fn claim(&mut self, cell: String, call: &str) -> Result<(), Unread> {
        match self.running.get(&cell) {
            Some(Some(owner)) if owner == call => return Ok(()),
            Some(None) => return Ok(()),
            Some(Some(_)) => {
                // Two calls at once: contested for good.
                let owner = self.running.insert(cell, None).flatten();
                self.strings -= owner.map_or(0, |owner| owner.capacity());
            }
            None => {
                let strings = cell.capacity() + call.len();
                self.room(self.running.len(), self.running.capacity(), strings)?;
                self.running.insert(cell, Some(call.to_owned()));
                self.strings += strings;
            }
        }
        self.fit()
    }

    /// The `wait` call `call` waits on `cell`.
    fn wait(&mut self, call: &str, cell: &str) -> Result<(), Unread> {
        if self.waits.contains_key(call) {
            return Ok(());
        }
        let strings = call.len() + cell.len();
        self.room(self.waits.len(), self.waits.capacity(), strings)?;
        self.waits.insert(call.to_owned(), cell.to_owned());
        self.strings += strings;
        self.fit()
    }

    /// The output of `call`, whose header is `header`.
    fn output(&mut self, call: &str, header: CellHeader) -> Result<(), Unread> {
        if let Some((waiter, cell)) = self.waits.remove_entry(call) {
            self.strings -= waiter.capacity() + cell.capacity();
            // A wait that saw its cell complete ends that cell's claim.
            if header == CellHeader::Completed
                && let Some(Some(owner)) = self.running.get(&cell)
            {
                self.strings -= owner.capacity();
                if let Some((key, _)) = self.running.remove_entry(&cell) {
                    self.strings -= key.capacity();
                }
            }
            return self.fit();
        }
        if let CellHeader::Running(cell) = header {
            return self.claim(cell, call);
        }
        Ok(())
    }

    /// A cell announced running by an output that pairs with no call: no
    /// owner can be known, so it is contested for good.
    fn poison(&mut self, cell: String) -> Result<(), Unread> {
        match self.running.get(&cell) {
            Some(None) => return Ok(()),
            Some(Some(_)) => {
                let owner = self.running.insert(cell, None).flatten();
                self.strings -= owner.map_or(0, |owner| owner.capacity());
            }
            None => {
                let strings = cell.capacity();
                self.room(self.running.len(), self.running.capacity(), strings)?;
                self.running.insert(cell, None);
                self.strings += strings;
            }
        }
        self.fit()
    }

    fn contested(&self, cell: &str) -> bool {
        matches!(self.running.get(cell), Some(None))
    }
}

/// What the rows read so far established.
#[derive(Debug)]
struct Rows {
    valid: bool,
    expected: Option<i64>,
    ordered: bool,
    first_row: bool,
    counts: HashMap<u64, Counts>,
    cells: Cells,
    open: Vec<Open>,
    found: Vec<Found>,
    broken: usize,
    entries: Reserved,
    /// The launches found and the slots of the open ones.
    bytes: Reserved,
}

/// One file's read, in progress.
#[derive(Debug)]
pub(super) struct SegmentScan {
    segment: Segment,
    thread: String,
    key: RandomState,
    allowance: Allowance,
    entries: Allowance,
    file: File,
    generation: SegmentGeneration,
    lines: Lines,
    rows: Rows,
    /// Its copies of the segment and thread.
    _held: Reserved,
}

impl SegmentScan {
    /// The bytes the read actually keeps apart from its call-count map
    /// (bounded by entries) and its fixed-size parts.
    #[cfg(test)]
    pub fn actual(&self) -> usize {
        let rows = &self.rows;
        let open: usize = rows
            .open
            .iter()
            .map(|open| open.launch_call_id.capacity() + open.child.capacity() + open.ack.actual())
            .sum();
        let cells = &rows.cells;
        let map = |map: &HashMap<String, Option<String>>| {
            source::table_actual(map.capacity(), PAIR)
                + map
                    .iter()
                    .map(|(key, value)| key.capacity() + value.as_ref().map_or(0, String::capacity))
                    .sum::<usize>()
        };
        let waits = source::table_actual(cells.waits.capacity(), PAIR)
            + cells
                .waits
                .iter()
                .map(|(key, value)| key.capacity() + value.capacity())
                .sum::<usize>();
        self.segment.bytes()
            + self.thread.capacity()
            + self.lines.actual()
            + rows.open.capacity() * std::mem::size_of::<Open>()
            + open
            + rows.found.capacity() * std::mem::size_of::<Found>()
            + rows.found.iter().map(Found::bytes).sum::<usize>()
            + map(&cells.running)
            + waits
    }

    /// Start reading the file of `segment` from its first byte. `allowance`
    /// holds its buffers and `entries` its map, both the thread's.
    pub fn start(
        segment: &Segment,
        thread: &str,
        key: &RandomState,
        allowance: &Allowance,
        entries: &Allowance,
        max_line: usize,
    ) -> Result<Self, ScanError> {
        let (file, generation) = source::open(&segment.path).map_err(ScanError::Source)?;
        // Only the generation the plan was made from: else plan again.
        if generation != segment.generation {
            return Err(ScanError::Source(Unread::Changed));
        }
        let over = |_| ScanError::OverLimit;
        let held = allowance
            .reserve(segment.bytes() + thread.len())
            .map_err(over)?;
        Ok(Self {
            segment: segment.clone(),
            thread: thread.to_owned(),
            key: key.clone(),
            allowance: allowance.clone(),
            entries: entries.clone(),
            file,
            generation,
            lines: Lines::new(max_line, allowance).map_err(over)?,
            rows: Rows {
                valid: true,
                expected: None,
                ordered: segment.header.ordinal.is_some(),
                first_row: true,
                counts: HashMap::new(),
                cells: Cells {
                    running: HashMap::new(),
                    waits: HashMap::new(),
                    strings: 0,
                    held: allowance.reserve(0).map_err(over)?,
                    entries: entries.reserve(0).map_err(over)?,
                },
                open: Vec::new(),
                found: Vec::new(),
                broken: 0,
                entries: entries.reserve(0).map_err(over)?,
                bytes: allowance.reserve(0).map_err(over)?,
            },
            _held: held,
        })
    }

    /// Read on within `budget`.
    pub fn advance(
        &mut self,
        budget: &mut Budget<'_>,
        programs: &[PathBuf],
        bounds: &ScanBounds,
    ) -> Result<Step, ScanError> {
        let len = u64::try_from(self.generation.length)
            .map_err(|_| ScanError::Source(Unread::Changed))?;
        while self.lines.next < len && self.rows.valid {
            let Some(chunk) =
                source::read_chunk(&self.file, self.lines.next, len - self.lines.next, budget)
                    .map_err(ScanError::Source)?
            else {
                return Ok(Step::Paused);
            };
            let rows = &mut self.rows;
            let (segment, thread, key, allowance) =
                (&self.segment, &self.thread, &self.key, &self.allowance);
            let fed = self.lines.feed(&chunk, &mut |offset, bytes| {
                rows.line(
                    offset, bytes, segment, thread, key, allowance, programs, bounds,
                )
            });
            match fed {
                Ok(()) => {}
                // A row too long to look at: the file's structure is not
                // known, so nothing in it is used.
                Err(Unread::TooLarge) => self.rows.valid = false,
                Err(Unread::Memory) => return Err(ScanError::OverLimit),
                Err(other) => return Err(ScanError::Source(other)),
            }
        }
        // Still exactly the generation that was read.
        let after = source::generation_of(&self.file).map_err(ScanError::Source)?;
        let named = source::stat(&self.segment.path).map_err(ScanError::Source)?;
        if after != self.generation || named != self.generation {
            return Err(ScanError::Source(Unread::Changed));
        }
        // A last line still being written: the prefix certifies nothing.
        if self.rows.valid && !self.lines.finished() {
            return Ok(Step::Unfinished);
        }
        let over = |_| ScanError::OverLimit;
        let found = std::mem::take(&mut self.rows.found);
        let chains = if self.rows.valid { found } else { Vec::new() };
        let mut bytes = std::mem::replace(
            &mut self.rows.bytes,
            self.allowance.reserve(0).map_err(over)?,
        );
        // The facts' own copy of the rollout, then exactly what they hold.
        bytes
            .grow(std::mem::size_of::<MemberFacts>() + self.segment.rollout.len())
            .map_err(over)?;
        let rollout = self.segment.rollout.clone();
        bytes
            .resize(MemberFacts::held(&rollout, &chains))
            .map_err(over)?;
        let facts = MemberFacts {
            rollout,
            generation: self.generation,
            complete: self.lines.complete,
            valid: self.rows.valid,
            counts: std::mem::take(&mut self.rows.counts),
            chains,
            broken: self.rows.broken,
            entries: std::mem::replace(
                &mut self.rows.entries,
                self.entries.reserve(0).map_err(over)?,
            ),
            bytes,
        };
        Ok(Step::Done(Box::new(facts)))
    }
}

impl Rows {
    /// One complete row at `offset`. `Err` only when the thread's allowance
    /// cannot hold what the row needs.
    #[allow(clippy::too_many_arguments)]
    fn line(
        &mut self,
        offset: u64,
        bytes: &[u8],
        segment: &Segment,
        thread: &str,
        key: &RandomState,
        allowance: &Allowance,
        programs: &[PathBuf],
        bounds: &ScanBounds,
    ) -> Result<(), Unread> {
        if !self.valid {
            return Ok(());
        }
        if offset == 0 {
            // The header the group was validated with.
            let _decoding =
                allowance.reserve(source::decoded_bound(bytes) + rows::row_bound(bytes))?;
            if super::group::header(bytes, thread).ok().as_ref() != Some(&segment.header) {
                self.valid = false;
            }
            self.expected = segment.header.ordinal.map(|ordinal| ordinal + 1);
            return Ok(());
        }
        // The row's structural fields, kept while the row is looked at.
        let _head = allowance.reserve(rows::row_bound(bytes))?;
        let Some(mut row) = rows::row(bytes) else {
            self.valid = false;
            return Ok(());
        };
        if row.kind.as_deref() == Some("session_meta") {
            self.valid = false;
            return Ok(());
        }
        // Actual ordinals, on every row.
        if self.ordered {
            if row.ordinal != self.expected {
                self.valid = false;
                return Ok(());
            }
            self.expected = self.expected.map(|ordinal| ordinal + 1);
        } else if self.first_row && row.ordinal.is_some() {
            self.ordered = true;
            self.expected = row.ordinal.map(|ordinal| ordinal + 1);
        } else if row.ordinal.is_some() {
            self.valid = false;
            return Ok(());
        }
        self.first_row = false;
        let (is_call, is_output) = (row.is_call(), row.is_output());
        let unpairable = row.unpairable_output();
        if (is_call || is_output) && !unpairable {
            let Some(id) = row.call_id() else {
                self.valid = false;
                return Ok(());
            };
            let hash = id_hash(key, id);
            let counts = match self.counts.get_mut(&hash) {
                Some(counts) => counts,
                None => {
                    self.entries.grow(1)?;
                    self.counts.entry(hash).or_default()
                }
            };
            *counts = counts.plus(if is_call {
                Counts {
                    calls: 1,
                    outputs: 0,
                }
            } else {
                Counts {
                    calls: 0,
                    outputs: 1,
                }
            });
        }
        let own = segment
            .header
            .inherited_below
            .is_none_or(|below| row.ordinal.is_some_and(|o| o >= below));
        // A native output without a call identifier pairs with nothing: it is
        // not counted, owns and releases no cell, and its own identifier,
        // name and body are never taken for a call. A cell it announces
        // running is unusable as proof; launches open now cannot tell it is
        // not theirs, so they end.
        if unpairable {
            if own
                && let Some(CellHeader::Running(cell)) = row
                    .payload
                    .as_mut()
                    .and_then(|payload| payload.output.take())
                    .map(|head| head.0)
            {
                self.cells.poison(cell)?;
            }
            self.broken += self.open.len();
            self.open.clear();
            return Ok(());
        }
        // Every own output's header: a cell it announces running, or a wait
        // it answers.
        if own && is_output {
            let header = row
                .payload
                .as_mut()
                .and_then(|payload| payload.output.take())
                .map(|head| head.0)
                .unwrap_or_default();
            self.cells
                .output(row.call_id().unwrap_or_default(), header)?;
        }
        let launching = own && row.is_exec();
        // A `wait` call names its cell in its arguments: decoded whole.
        let waiting = own
            && is_call
            && row.payload_kind() == Some("function_call")
            && row.payload.as_ref().and_then(|p| p.name.as_deref()) == Some("wait");
        if self.open.is_empty() && !launching && !waiting {
            return Ok(());
        }
        // The decoded row is a temporary copy: reserved before decoding.
        let _decoding = allowance.reserve(source::decoded_bound(bytes))?;
        let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
            self.valid = false;
            return Ok(());
        };
        let item = match rows::item(&value) {
            Ok(item) => item,
            Err(()) => {
                self.valid = false;
                return Ok(());
            }
        };
        if waiting && let Some(Item::Call { call_id, input, .. }) = &item {
            let _arguments = allowance.reserve(source::decoded_bound(input.as_bytes()))?;
            if let Ok(serde_json::Value::Object(arguments)) = serde_json::from_str::<Value>(input)
                && let Some(cell) = arguments.get("cell_id").and_then(Value::as_str)
            {
                self.cells.wait(call_id, cell)?;
            }
        }
        // Every open launch sees the row.
        let size = bytes.len() as u64 + 1;
        let mut index = 0;
        while index < self.open.len() {
            let open = &mut self.open[index];
            open.window_bytes += size;
            open.window_lines += 1;
            let step = if open.window_bytes > bounds.max_window_bytes
                || open.window_lines > bounds.max_window_lines
            {
                Err(Broken::TooLong)
            } else {
                match &item {
                    Some(item) => open.ack.feed(item),
                    None => Ok(None),
                }
            };
            match step {
                Ok(None) => index += 1,
                Ok(Some(Acknowledged { call, handle })) => {
                    let done = self.open.swap_remove(index);
                    let (true, true) = (
                        call_token(&call),
                        done.ack.own.iter().all(|id| call_token(id)),
                    ) else {
                        self.broken += 1;
                        continue;
                    };
                    if handle.as_ref().is_some_and(|handle| handle.len() > 32) {
                        self.broken += 1;
                        continue;
                    }
                    // What the launch keeps, reserved before it is made.
                    self.bytes.grow(
                        done.launch_call_id.capacity()
                            + done.child.capacity()
                            + call.capacity()
                            + handle.as_ref().map_or(0, String::capacity)
                            + done.ack.own.len() * 8,
                    )?;
                    source::grow_items(&mut self.found, &mut self.bytes)?;
                    let found = Found {
                        launch_call_id: done.launch_call_id,
                        launch_op: done.launch_op,
                        child: done.child,
                        acknowledgment_call_id: call,
                        handle,
                        launch_offset: done.launch_offset,
                        acknowledgment_offset: offset,
                        launch_ordinal: done.launch_ordinal,
                        acknowledgment_ordinal: row.ordinal,
                        ids: done.ack.own.iter().map(|id| id_hash(key, id)).collect(),
                    };
                    self.found.push(found);
                }
                Err(Broken::Memory) => return Err(Unread::Memory),
                Err(_) => {
                    self.open.swap_remove(index);
                    self.broken += 1;
                }
            }
        }
        self.disown();
        if !launching {
            return Ok(());
        }
        // A launch: an `exec` cell's `exec_command` operation that is one
        // literal explicit-session Claude print command. Other operations
        // only keep their places.
        let Some(Item::Call { call_id, input, .. }) = &item else {
            return Ok(());
        };
        if rows::timestamp(&value).is_none() || !call_token(call_id) {
            return Ok(());
        }
        let Ok(ops) = cell::parse_operations(input) else {
            return Ok(());
        };
        for (op, operation) in ops.iter().enumerate() {
            if operation.tool != Tool::ExecCommand {
                continue;
            }
            let Some(launch) = operation
                .str("cmd")
                .and_then(|cmd| command::launch(cmd, programs))
            else {
                continue;
            };
            if !launch_args_allowed(operation) || self.open.len() >= bounds.max_open.min(64) {
                self.broken += 1;
                continue;
            }
            let Ok(ack) = Ack::new(call_id, op, ops.len(), allowance) else {
                return Err(Unread::Memory);
            };
            let held = allowance.reserve(call_id.len() + launch.child.capacity())?;
            source::grow_items(&mut self.open, &mut self.bytes)?;
            self.open.push(Open {
                ack,
                launch_call_id: call_id.clone(),
                launch_op: u32::try_from(op).unwrap_or(u32::MAX),
                child: launch.child,
                launch_offset: offset,
                launch_ordinal: row.ordinal,
                window_bytes: 0,
                window_lines: 0,
                _held: held,
            });
        }
        Ok(())
    }

    /// Break every open launch whose running cell two calls claim.
    fn disown(&mut self) {
        // At most `MAX_OPEN_CHAINS` (64) are open: one bit each.
        let mut ambiguous = 0_u64;
        for index in 0..self.open.len().min(64) {
            if self.open[index]
                .ack
                .cell()
                .is_some_and(|cell| self.cells.contested(cell))
            {
                ambiguous |= 1 << index;
            }
        }
        if ambiguous == 0 {
            return;
        }
        let mut index = 0;
        self.open.retain(|_| {
            let keep = ambiguous & (1 << index) == 0;
            index += 1;
            keep
        });
        self.broken += ambiguous.count_ones() as usize;
    }
}
