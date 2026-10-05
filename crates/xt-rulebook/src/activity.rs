//! Bounded, read-only snapshot of recorded rule activity.
//!
//! The MemHub plugin appends one JSON line per recorded rule fire to
//! `<root>/ledger/fires.jsonl` and marks the row shape in
//! `<root>/ledger/schema_version`. This module reads one bounded snapshot of
//! that ledger and returns typed structural metadata only.
//!
//! What a snapshot can and cannot say:
//!
//! - It describes rows *recorded* in this one source, not all rule activity.
//!   The plugin can fail to record (for example when its lock dependency is
//!   unavailable), so an empty snapshot means "0 recorded rows in this source
//!   snapshot", never "no rules fired".
//! - [`FireMode`] is the recorded delivery mode. `Gate` does not mean the call
//!   was blocked (blocked and overridden gate calls are recorded alike),
//!   `Advise` does not mean the advice was followed, and `Suppressed` is a
//!   recorded suppression, not a delivered fire.
//! - The oldest accepted timestamp is only the earliest row observed in the
//!   bounded snapshot. It is not a coverage start: the ledger has no retention
//!   or beginning-of-coverage guarantee.
//! - Counts are [`Precision::Exact`] only for a complete, clean scan of the
//!   whole file; any bound, malformed row, conflict or unfinished line makes
//!   them a lower bound. An interrupted or changed read returns no counts.
//! - An [`ObservedGroup`] is a `(rulebook_id, rule_id)` pair seen in the
//!   snapshot's in-window records, not an active or distinct rule. A row
//!   without a rulebook is an unscoped group of its own. Group counts
//!   describe the same bounded scan and carry the same precision.
//!
//! Privacy: only the whitelisted fields of [`FireRecord`] are ever
//! materialized. `excerpt`, `override_reason`, `source_message_id`,
//! `dedup_key`, `raw_matches_before_fire` and unknown keys are skipped by the
//! parser without being stored, and no raw line, JSON text or parser message
//! is kept in any result, error or log (this module does not log).
//!
//! Scope: one explicitly selected root, no discovery, no writes, no caching,
//! no watching, no subprocess and no network. Session identities are the raw
//! recorded host/native values; no normalization or index linking happens
//! here. Callers own scheduling (one in-flight read per source) and must run
//! the read off any latency-sensitive thread.

mod record;
mod source;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use jiff::Timestamp;

pub use record::{FireMode, FireRecord, RuleVersion};
pub use source::{PathProblem, SourcePart};

use record::Reject;
use source::{Budget, Captured};

/// Default rulebook root relative to the user's home directory.
pub const DEFAULT_ROOT_UNDER_HOME: &str = ".config/memhub-plugin/rulebook";

/// The only ledger row shape this reader accepts.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 2;

/// The default source root under `home`. The caller supplies `home`; this
/// module never reads the process environment to find it.
pub fn default_root(home: &Path) -> PathBuf {
    home.join(DEFAULT_ROOT_UNDER_HOME)
}

/// Resource bounds for one read. The defaults are the maxima; every setter
/// can only tighten a bound, never loosen it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadLimits {
    tail_bytes: u64,
    lines: usize,
    line_bytes: usize,
    value_bytes: usize,
    rows: usize,
    groups: usize,
    deadline: Duration,
}

impl ReadLimits {
    /// At most this many bytes are read from the end of the ledger.
    pub const MAX_TAIL_BYTES: u64 = 4 << 20;
    /// At most this many (latest) complete physical lines are examined.
    pub const MAX_LINES: usize = 10_000;
    /// A longer line is rejected without being parsed.
    pub const MAX_LINE_BYTES: usize = 64 << 10;
    /// A longer string value is rejected (never truncated).
    pub const MAX_VALUE_BYTES: usize = 512;
    /// At most this many rows are returned for presentation.
    pub const MAX_ROWS: usize = 100;
    /// At most this many observed groups are returned for presentation.
    pub const MAX_GROUPS: usize = 100;
    /// A read that has not finished by this deadline is interrupted.
    pub const MAX_DEADLINE: Duration = Duration::from_secs(1);

    pub fn tail_bytes(&self) -> u64 {
        self.tail_bytes
    }
    pub fn lines(&self) -> usize {
        self.lines
    }
    pub fn line_bytes(&self) -> usize {
        self.line_bytes
    }
    pub fn value_bytes(&self) -> usize {
        self.value_bytes
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn groups(&self) -> usize {
        self.groups
    }
    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    #[must_use]
    pub fn tighten_tail_bytes(mut self, bytes: u64) -> Self {
        self.tail_bytes = self.tail_bytes.min(bytes);
        self
    }
    #[must_use]
    pub fn tighten_lines(mut self, lines: usize) -> Self {
        self.lines = self.lines.min(lines);
        self
    }
    #[must_use]
    pub fn tighten_line_bytes(mut self, bytes: usize) -> Self {
        self.line_bytes = self.line_bytes.min(bytes);
        self
    }
    #[must_use]
    pub fn tighten_value_bytes(mut self, bytes: usize) -> Self {
        self.value_bytes = self.value_bytes.min(bytes);
        self
    }
    #[must_use]
    pub fn tighten_rows(mut self, rows: usize) -> Self {
        self.rows = self.rows.min(rows);
        self
    }
    #[must_use]
    pub fn tighten_groups(mut self, groups: usize) -> Self {
        self.groups = self.groups.min(groups);
        self
    }
    #[must_use]
    pub fn tighten_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = self.deadline.min(deadline);
        self
    }
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self {
            tail_bytes: Self::MAX_TAIL_BYTES,
            lines: Self::MAX_LINES,
            line_bytes: Self::MAX_LINE_BYTES,
            value_bytes: Self::MAX_VALUE_BYTES,
            rows: Self::MAX_ROWS,
            groups: Self::MAX_GROUPS,
            deadline: Self::MAX_DEADLINE,
        }
    }
}

/// A half-open `[start, end)` window over `fired_at`, anchored once by the
/// caller for the whole request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivityWindow {
    start: Timestamp,
    end: Timestamp,
}

impl ActivityWindow {
    /// `None` unless `start < end`.
    pub fn new(start: Timestamp, end: Timestamp) -> Option<Self> {
        (start < end).then_some(Self { start, end })
    }
    pub fn start(&self) -> Timestamp {
        self.start
    }
    pub fn end(&self) -> Timestamp {
        self.end
    }
    pub fn contains(&self, at: Timestamp) -> bool {
        self.start <= at && at < self.end
    }
}

/// The outcome of one read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivityRead {
    /// No usable source. Never an empty source.
    Unavailable(Unavailable),
    /// The ledger was replaced, shrunk or removed while it was read; nothing
    /// from this read is usable. A manual retry takes a fresh snapshot.
    SourceChanged(SourceChange),
    /// Cancelled or past the deadline; no partial counts are returned.
    Interrupted(Interruption),
    Snapshot(Box<ActivitySnapshot>),
}

/// Why no snapshot could be taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// The selected root is not an absolute path.
    RootNotAbsolute,
    /// A path under the root is missing, unsafe or unreadable.
    Path {
        part: SourcePart,
        problem: PathProblem,
    },
    /// The schema marker is not a plain decimal integer line.
    SchemaMalformed,
    /// The schema marker names a row shape other than
    /// [`SUPPORTED_SCHEMA_VERSION`].
    SchemaUnsupported { found: u64 },
}

/// How the ledger changed between capture and verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceChange {
    /// The open file is now shorter than the captured end (truncation).
    Shrunk,
    /// A different object (or a link) now stands at the ledger path.
    Replaced,
    /// Nothing stands at the ledger path any more.
    Removed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interruption {
    Cancelled,
    DeadlineExceeded,
}

/// One bounded snapshot of recorded rule activity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivitySnapshot {
    /// Canonical form of the selected root. Other roots are not covered.
    pub root: PathBuf,
    pub schema_version: u32,
    pub read_at: Timestamp,
    pub window: Option<ActivityWindow>,
    pub coverage: SnapshotCoverage,
    pub counts: ActivityCounts,
    /// Latest accepted in-window rows, newest `fired_at` first, ties broken
    /// by descending `fire_id`; at most [`ReadLimits::rows`].
    pub rows: Vec<FireRecord>,
    /// Groups of *all* accepted in-window rows (not only the returned
    /// ones), latest observed first; at most [`ReadLimits::groups`].
    pub groups: Vec<ObservedGroup>,
    /// Identity of the file this snapshot came from; internal only.
    identity: source::FileIdentity,
}

impl ActivitySnapshot {
    /// Whether `other` was taken from the same file object (device/inode).
    /// Snapshots of different objects must never be merged.
    pub fn same_source_object(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

/// What part of the ledger the snapshot covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotCoverage {
    /// Ledger size captured at open; no byte past it was read.
    pub captured_len: u64,
    /// Byte range of the complete lines actually examined.
    pub scanned: Range<u64>,
    /// The tail byte bound excluded earlier bytes.
    pub byte_bound_reached: bool,
    /// The line bound excluded earlier complete lines.
    pub line_bound_reached: bool,
    /// More in-window rows were accepted than returned.
    pub row_bound_reached: bool,
    /// More observed groups were found than returned. Returned groups keep
    /// their full counts.
    pub group_bound_reached: bool,
    /// The read started inside a line, which was dropped.
    pub leading_partial_dropped: bool,
    /// The captured end fell inside an unfinished line, which was dropped.
    pub trailing_partial_dropped: bool,
    /// The file grew after capture; those bytes are newer than this snapshot.
    pub grew_after_capture: bool,
    /// Earliest accepted `fired_at` observed. Not a coverage start.
    pub oldest_observed: Option<Timestamp>,
    /// Latest accepted `fired_at` observed.
    pub newest_observed: Option<Timestamp>,
}

/// Counts over the examined lines of one snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityCounts {
    /// Complete physical lines examined.
    pub lines: u64,
    /// Whitespace-only lines (not records, not malformed).
    pub blank_lines: u64,
    pub malformed: MalformedCounts,
    /// Rows that passed validation, before deduplication.
    pub valid_rows: u64,
    /// Distinct `fire_id`s with consistent metadata.
    pub distinct: u64,
    /// Extra rows identical to an earlier row of the same `fire_id`.
    pub duplicate_rows: u64,
    /// `fire_id`s recorded with differing metadata; excluded from rows and
    /// mode counts.
    pub conflicted_ids: u64,
    /// All rows belonging to conflicted `fire_id`s.
    pub conflicted_rows: u64,
    /// Distinct consistent records inside the window, by recorded mode.
    pub in_window: ModeCounts,
    /// Distinct `(rulebook_id, rule_id)` groups among those records,
    /// including groups beyond the presentation bound.
    pub observed_groups: u64,
    pub precision: Precision,
}

/// Per-mode counts of distinct consistent records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModeCounts {
    pub advise: u64,
    pub gate: u64,
    pub suppressed: u64,
    pub unrecognized: u64,
}

impl ModeCounts {
    fn record(&mut self, mode: &FireMode) {
        match mode {
            FireMode::Advise => self.advise += 1,
            FireMode::Gate => self.gate += 1,
            FireMode::Suppressed => self.suppressed += 1,
            FireMode::Unrecognized(_) => self.unrecognized += 1,
        }
    }

    /// Recorded fires: known `advise` plus `gate` rows only. Suppressed and
    /// unrecognized modes are reported separately and excluded.
    pub fn recorded_fires(&self) -> u64 {
        self.advise + self.gate
    }
    pub fn total(&self) -> u64 {
        self.advise + self.gate + self.suppressed + self.unrecognized
    }
}

/// The in-window records of one `(rulebook_id, rule_id)` pair.
///
/// Identities are the raw recorded values. A missing `rulebook_id` makes an
/// unscoped group that never merges with a scoped one, and the same
/// `rule_id` under two rulebooks is two groups. A group spans every
/// recorded mode and rule version; it names no current mode, version,
/// outcome or lifecycle state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedGroup {
    pub rulebook_id: Option<String>,
    pub rule_id: String,
    /// Distinct consistent in-window records, by recorded mode.
    pub modes: ModeCounts,
    /// Latest in-window `fired_at` of the group.
    pub latest_observed: Timestamp,
}

/// Rejected lines, by reason. No line content is retained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MalformedCounts {
    /// Longer than [`ReadLimits::line_bytes`]; not parsed.
    pub oversize_line: u64,
    /// Not a single valid UTF-8 JSON value.
    pub invalid_json: u64,
    /// Valid JSON but not an object with the required, well-typed fields.
    pub invalid_shape: u64,
    /// `fired_at` is not an instant with an offset.
    pub invalid_timestamp: u64,
    /// A whitelisted string longer than [`ReadLimits::value_bytes`].
    pub oversize_value: u64,
}

impl MalformedCounts {
    pub fn total(&self) -> u64 {
        self.oversize_line
            + self.invalid_json
            + self.invalid_shape
            + self.invalid_timestamp
            + self.oversize_value
    }
}

/// Whether the counts are exact for this source snapshot.
///
/// `Exact` means the whole captured file was examined cleanly: it is exact
/// about *recorded rows in this snapshot*, never about all rule activity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    Exact,
    LowerBound,
}

/// Read one bounded snapshot of the ledger under `root`.
///
/// `cancel` is polled between chunks and batches of lines; returning `true`
/// interrupts the read. The deadline in `limits` is measured from this call.
pub fn read_activity(
    root: &Path,
    limits: &ReadLimits,
    window: Option<ActivityWindow>,
    cancel: &dyn Fn() -> bool,
) -> ActivityRead {
    let checkpoints = &mut Checkpoints {
        after_capture: &mut || {},
        after_assembly: &mut || {},
    };
    read_activity_with(root, limits, window, cancel, checkpoints)
}

/// Test hooks at fixed points of a read. `after_capture` runs after the
/// bytes are captured and before the file is re-verified, so a test can
/// mutate the source; `after_assembly` runs after the snapshot is assembled
/// and before the final budget check, so a test can cancel or outlast the
/// deadline there.
struct Checkpoints<'a> {
    after_capture: &'a mut dyn FnMut(),
    after_assembly: &'a mut dyn FnMut(),
}

fn read_activity_with(
    root: &Path,
    limits: &ReadLimits,
    window: Option<ActivityWindow>,
    cancel: &dyn Fn() -> bool,
    checkpoints: &mut Checkpoints<'_>,
) -> ActivityRead {
    let read_at = Timestamp::now();
    let budget = Budget::new(Instant::now() + limits.deadline, cancel);
    match snapshot(root, limits, window, read_at, &budget, checkpoints) {
        Ok(snapshot) => ActivityRead::Snapshot(Box::new(snapshot)),
        Err(stop) => stop.into(),
    }
}

/// Why a read stopped without a snapshot.
enum Stop {
    Unavailable(Unavailable),
    Changed(SourceChange),
    Interrupted(Interruption),
}

impl From<Stop> for ActivityRead {
    fn from(stop: Stop) -> Self {
        match stop {
            Stop::Unavailable(reason) => ActivityRead::Unavailable(reason),
            Stop::Changed(change) => ActivityRead::SourceChanged(change),
            Stop::Interrupted(why) => ActivityRead::Interrupted(why),
        }
    }
}

fn snapshot(
    root: &Path,
    limits: &ReadLimits,
    window: Option<ActivityWindow>,
    read_at: Timestamp,
    budget: &Budget<'_>,
    checkpoints: &mut Checkpoints<'_>,
) -> Result<ActivitySnapshot, Stop> {
    budget.check()?;
    let root = source::resolve_root(root)?;
    let ledger_dir = source::ledger_dir(&root)?;
    let schema_version = source::read_schema_marker(&ledger_dir)?;
    budget.check()?;
    let ledger = source::open_ledger(&ledger_dir)?;
    let captured = ledger.capture(limits.tail_bytes, budget)?;
    (checkpoints.after_capture)();
    let grew_after_capture = ledger.verify(captured.len)?;
    let lines = complete_lines(&captured, limits.lines);
    let scan = scan(&lines, limits, budget)?;
    let snapshot = scan.into_snapshot(Assembly {
        root,
        schema_version,
        read_at,
        window,
        max_rows: limits.rows,
        max_groups: limits.groups,
        captured: &captured,
        lines: &lines,
        grew_after_capture,
        identity: ledger.identity(),
    });
    // Assembly (aggregation, sorting) takes time too: a deadline or
    // cancellation that arrives during it still yields no snapshot.
    (checkpoints.after_assembly)();
    budget.check()?;
    Ok(snapshot)
}

/// The complete lines of a captured tail, after bounds.
struct Lines<'a> {
    /// Examined lines without their terminating newline.
    lines: Vec<&'a [u8]>,
    scanned: Range<u64>,
    line_bound_reached: bool,
    leading_partial_dropped: bool,
    trailing_partial_dropped: bool,
}

/// Split a captured tail into complete lines: drop a line cut by the tail
/// start, drop an unfinished final line, then keep only the latest
/// `max_lines` lines.
fn complete_lines(captured: &Captured, max_lines: usize) -> Lines<'_> {
    let bytes = captured.bytes.as_slice();
    let leading_partial_dropped = captured.starts_mid_line;
    let trailing_partial_dropped = bytes.last().is_some_and(|&b| b != b'\n');
    let complete_end = match bytes.iter().rposition(|&b| b == b'\n') {
        Some(newline) => newline + 1,
        None => 0,
    };
    let complete_start = if leading_partial_dropped {
        match bytes[..complete_end].iter().position(|&b| b == b'\n') {
            Some(newline) => newline + 1,
            None => complete_end,
        }
    } else {
        0
    };
    let region = &bytes[complete_start..complete_end];
    // Walk back from the end and keep at most one line past the bound, so
    // a tail of tiny lines never builds a slice per newline.
    let mut lines: Vec<&[u8]> = match region.split_last() {
        Some((_, body)) => body
            .rsplit(|&b| b == b'\n')
            .take(max_lines.saturating_add(1))
            .collect(),
        None => Vec::new(),
    };
    let line_bound_reached = lines.len() > max_lines;
    lines.truncate(max_lines);
    lines.reverse();
    let examined: usize = lines.iter().map(|line| line.len() + 1).sum();
    let offset = captured.start;
    let end = offset + to_u64(complete_end);
    Lines {
        scanned: end - to_u64(examined)..end,
        lines,
        line_bound_reached,
        leading_partial_dropped,
        trailing_partial_dropped,
    }
}

/// Lines between cancellation/deadline polls while parsing.
const LINES_PER_POLL: usize = 256;

/// One `fire_id`'s observations in a snapshot.
struct Slot {
    record: FireRecord,
    rows: u64,
    conflicted: bool,
}

/// Everything besides the scan that a snapshot is assembled from.
struct Assembly<'a> {
    root: PathBuf,
    schema_version: u32,
    read_at: Timestamp,
    window: Option<ActivityWindow>,
    max_rows: usize,
    max_groups: usize,
    captured: &'a Captured,
    lines: &'a Lines<'a>,
    grew_after_capture: bool,
    identity: source::FileIdentity,
}

struct Scan {
    blank_lines: u64,
    malformed: MalformedCounts,
    valid_rows: u64,
    slots: HashMap<String, Slot>,
}

fn scan(lines: &Lines<'_>, limits: &ReadLimits, budget: &Budget<'_>) -> Result<Scan, Stop> {
    let mut scan = Scan {
        blank_lines: 0,
        malformed: MalformedCounts::default(),
        valid_rows: 0,
        slots: HashMap::new(),
    };
    for (index, line) in lines.lines.iter().enumerate() {
        if index % LINES_PER_POLL == 0 {
            budget.check()?;
        }
        if line.len() > limits.line_bytes {
            scan.malformed.oversize_line += 1;
            continue;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            scan.blank_lines += 1;
            continue;
        }
        match record::parse(line, limits.value_bytes) {
            Ok(record) => scan.accept(record),
            Err(Reject::InvalidJson) => scan.malformed.invalid_json += 1,
            Err(Reject::InvalidShape) => scan.malformed.invalid_shape += 1,
            Err(Reject::InvalidTimestamp) => scan.malformed.invalid_timestamp += 1,
            Err(Reject::OversizeValue) => scan.malformed.oversize_value += 1,
        }
    }
    budget.check()?;
    Ok(scan)
}

impl Scan {
    /// Deduplicate by `fire_id`. Rows are not an update protocol: identical
    /// metadata counts once, differing metadata marks the id conflicted.
    fn accept(&mut self, record: FireRecord) {
        self.valid_rows += 1;
        match self.slots.entry(record.fire_id.clone()) {
            Entry::Vacant(vacant) => {
                vacant.insert(Slot {
                    record,
                    rows: 1,
                    conflicted: false,
                });
            }
            Entry::Occupied(mut occupied) => {
                let slot = occupied.get_mut();
                slot.rows += 1;
                if slot.record != record {
                    slot.conflicted = true;
                }
            }
        }
    }

    fn into_snapshot(self, assembly: Assembly<'_>) -> ActivitySnapshot {
        let Assembly {
            root,
            schema_version,
            read_at,
            window,
            max_rows,
            max_groups,
            captured,
            lines,
            grew_after_capture,
            identity,
        } = assembly;
        let mut distinct = 0;
        let mut duplicate_rows = 0;
        let mut conflicted_ids = 0;
        let mut conflicted_rows = 0;
        let mut in_window = ModeCounts::default();
        let mut oldest_observed: Option<Timestamp> = None;
        let mut newest_observed: Option<Timestamp> = None;
        let mut rows = Vec::new();
        for slot in self.slots.into_values() {
            if slot.conflicted {
                conflicted_ids += 1;
                conflicted_rows += slot.rows;
                continue;
            }
            distinct += 1;
            duplicate_rows += slot.rows - 1;
            let at = slot.record.fired_at;
            oldest_observed = Some(oldest_observed.map_or(at, |t| t.min(at)));
            newest_observed = Some(newest_observed.map_or(at, |t| t.max(at)));
            if window.is_some_and(|w| !w.contains(at)) {
                continue;
            }
            in_window.record(&slot.record.mode);
            rows.push(slot.record);
        }
        // Grouped before the presentation bound: groups cover every
        // accepted in-window row, not only the returned ones.
        let groups = group(&rows, max_groups);
        rows.sort_unstable_by(|a, b| (b.fired_at, &b.fire_id).cmp(&(a.fired_at, &a.fire_id)));
        let row_bound_reached = rows.len() > max_rows;
        rows.truncate(max_rows);

        let byte_bound_reached = captured.start > 0;
        let exact = lines.scanned.start == 0
            && !lines.line_bound_reached
            && !lines.trailing_partial_dropped
            && self.malformed.total() == 0
            && conflicted_ids == 0;
        ActivitySnapshot {
            root,
            schema_version,
            read_at,
            window,
            coverage: SnapshotCoverage {
                captured_len: captured.len,
                scanned: lines.scanned.clone(),
                byte_bound_reached,
                line_bound_reached: lines.line_bound_reached,
                row_bound_reached,
                group_bound_reached: groups.bound_reached,
                leading_partial_dropped: lines.leading_partial_dropped,
                trailing_partial_dropped: lines.trailing_partial_dropped,
                grew_after_capture,
                oldest_observed,
                newest_observed,
            },
            counts: ActivityCounts {
                lines: to_u64(lines.lines.len()),
                blank_lines: self.blank_lines,
                malformed: self.malformed,
                valid_rows: self.valid_rows,
                distinct,
                duplicate_rows,
                conflicted_ids,
                conflicted_rows,
                in_window,
                observed_groups: groups.total,
                precision: if exact {
                    Precision::Exact
                } else {
                    Precision::LowerBound
                },
            },
            rows,
            groups: groups.groups,
            identity,
        }
    }
}

/// Observed groups of a snapshot, after the presentation bound.
struct Groups {
    groups: Vec<ObservedGroup>,
    /// Every group, including those past the bound.
    total: u64,
    bound_reached: bool,
}

/// Group `records` by `(rulebook_id, rule_id)` and keep the `max_groups`
/// latest observed, ties broken by ascending `(rulebook_id, rule_id)` with
/// unscoped groups first.
fn group(records: &[FireRecord], max_groups: usize) -> Groups {
    let mut by_key: HashMap<(Option<&str>, &str), (ModeCounts, Timestamp)> = HashMap::new();
    for record in records {
        let key = (record.rulebook_id.as_deref(), record.rule_id.as_str());
        let (modes, latest) = by_key
            .entry(key)
            .or_insert((ModeCounts::default(), record.fired_at));
        modes.record(&record.mode);
        *latest = (*latest).max(record.fired_at);
    }
    let total = to_u64(by_key.len());
    let mut keyed: Vec<_> = by_key.into_iter().collect();
    keyed.sort_unstable_by(|(a_key, (_, a_at)), (b_key, (_, b_at))| {
        b_at.cmp(a_at).then_with(|| a_key.cmp(b_key))
    });
    let bound_reached = keyed.len() > max_groups;
    keyed.truncate(max_groups);
    let groups = keyed
        .into_iter()
        .map(
            |((rulebook_id, rule_id), (modes, latest_observed))| ObservedGroup {
                rulebook_id: rulebook_id.map(str::to_owned),
                rule_id: rule_id.to_owned(),
                modes,
                latest_observed,
            },
        )
        .collect();
    Groups {
        groups,
        total,
        bound_reached,
    }
}

fn to_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}
