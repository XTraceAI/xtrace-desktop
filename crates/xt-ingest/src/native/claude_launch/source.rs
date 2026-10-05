//! Reading the named local files: opened for reading only, without following
//! an alias, as a regular file with one link owned by this user, with its
//! generation (device, inode, length, modification and change times) taken
//! from the open descriptor. Bytes stay in memory; nothing here prints, logs
//! or stores them.
//!
//! Every read of a pass asks one [`Budget`] — bytes, directory entries, a
//! deadline and a cancel shared by everything the pass does — how much it may
//! read before reading, and is charged what it actually read; a line longer
//! than one pass allows is kept partly read and continued next pass. Every
//! buffer a thread's work keeps between reads is reserved from that thread's
//! [`Allowance`] at its actual capacity before it grows, and given back when
//! dropped; decoding one row reserves [`decoded_bound`] first.

use super::super::readers_cli::CancelToken;
use std::{
    fs::{File, Metadata, OpenOptions},
    os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use xt_store::claude_launch::SegmentGeneration;

/// The most bytes one read takes at a time.
pub(super) const CHUNK: u64 = 256 * 1024;
/// The first read of a single line; each further read of it doubles, up to
/// [`CHUNK`], so a short line costs about its own length.
const LINE_STEP: u64 = 4 * 1024;

/// How much the next read of a line takes, after `held` bytes of it.
fn line_step(held: usize) -> u64 {
    (LINE_STEP.max(held as u64)).min(CHUNK)
}

/// Why a file could not be read as the file that was observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unread {
    Missing,
    /// An alias, not a regular file, more than one link or another owner.
    Alias,
    Unreadable,
    /// It changed, was replaced or shrank while it was read.
    Changed,
    /// A line or the file is longer than its bound.
    TooLarge,
    /// Keeping it would pass the thread's memory allowance.
    Memory,
    /// Its last line is still being written: nothing of it is used until it
    /// is complete.
    Unfinished,
}

/// One pass's shared limits: source bytes, directory entries, a deadline
/// and a cancel.
pub(super) struct Budget<'a> {
    deadline: Instant,
    bytes_left: u64,
    entries_left: u64,
    cancel: Option<&'a CancelToken>,
    /// Bytes actually read so far.
    pub spent: u64,
    /// Directory entries taken so far.
    pub entries: u64,
}

impl<'a> Budget<'a> {
    pub fn new(bytes: u64, time: Duration, cancel: Option<&'a CancelToken>) -> Self {
        Self {
            deadline: Instant::now() + time,
            bytes_left: bytes,
            entries_left: u64::MAX,
            cancel,
            spent: 0,
            entries: 0,
        }
    }

    /// The same budget, looking at no more than `entries` directory entries.
    pub fn with_entries(self, entries: u64) -> Self {
        Self {
            entries_left: entries,
            ..self
        }
    }

    /// Take one directory entry, when the pass may still look at one: not
    /// cancelled, within its time and its entries.
    pub fn entry(&mut self) -> bool {
        if self.entries_left == 0 || self.out_of_time() {
            return false;
        }
        self.entries_left -= 1;
        self.entries += 1;
        true
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.is_some_and(CancelToken::is_cancelled)
    }

    /// Whether the pass's time is up or it was cancelled.
    pub fn out_of_time(&self) -> bool {
        self.cancelled() || Instant::now() >= self.deadline
    }

    /// Whether nothing more may be read this pass.
    pub fn exhausted(&self) -> bool {
        self.bytes_left == 0 || self.out_of_time()
    }

    /// How many of `want` bytes may be read now, at most one chunk.
    pub fn grant(&self, want: u64) -> u64 {
        if self.exhausted() {
            0
        } else {
            want.min(self.bytes_left).min(CHUNK)
        }
    }

    /// Count bytes actually read.
    pub fn charge(&mut self, bytes: u64) {
        self.bytes_left = self.bytes_left.saturating_sub(bytes);
        self.spent += bytes;
    }
}

#[cfg(test)]
thread_local! {
    /// Every byte any read here returned, for tests that compare it with
    /// what the budget was charged.
    pub(super) static READ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Read exactly the granted part of `want` bytes at `offset`, charged to the
/// budget. `None` when the budget grants nothing now.
pub(super) fn read_chunk(
    file: &File,
    offset: u64,
    want: u64,
    budget: &mut Budget<'_>,
) -> Result<Option<Vec<u8>>, Unread> {
    let granted = budget.grant(want);
    if granted == 0 {
        return Ok(None);
    }
    let mut chunk = vec![0_u8; usize::try_from(granted).map_err(|_| Unread::TooLarge)?];
    let read = loop {
        match file.read_at(&mut chunk, offset) {
            Ok(read) => break read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(Unread::Unreadable),
        }
    };
    #[cfg(test)]
    READ.with(|total| total.set(total.get() + read as u64));
    budget.charge(read as u64);
    if read == 0 {
        // Shorter than the length that was observed: it shrank.
        return Err(Unread::Changed);
    }
    chunk.truncate(read);
    Ok(Some(chunk))
}

/// A thread's memory allowance for the buffers its work keeps: a counter
/// shared by every holder, reserved before a buffer grows.
#[derive(Clone, Debug)]
pub(super) struct Allowance {
    used: Arc<AtomicUsize>,
    limit: usize,
}

/// Bytes reserved from an allowance, given back when dropped.
#[derive(Debug)]
pub(super) struct Reserved {
    allowance: Allowance,
    bytes: usize,
}

impl Allowance {
    pub fn new(limit: usize) -> Self {
        Self {
            used: Arc::new(AtomicUsize::new(0)),
            limit,
        }
    }

    #[cfg(test)]
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    /// Reserve `bytes`, or refuse without reserving anything.
    pub fn reserve(&self, bytes: usize) -> Result<Reserved, Unread> {
        let mut reserved = Reserved {
            allowance: self.clone(),
            bytes: 0,
        };
        reserved.grow(bytes)?;
        Ok(reserved)
    }
}

impl Reserved {
    /// Reserve `extra` more, or refuse without reserving it.
    pub fn grow(&mut self, extra: usize) -> Result<(), Unread> {
        let limit = self.allowance.limit;
        self.allowance
            .used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(extra).filter(|total| *total <= limit)
            })
            .map_err(|_| Unread::Memory)?;
        self.bytes += extra;
        Ok(())
    }

    /// Hold exactly `bytes`: shrinking always succeeds, growing may refuse.
    pub fn resize(&mut self, bytes: usize) -> Result<(), Unread> {
        if bytes > self.bytes {
            self.grow(bytes - self.bytes)
        } else {
            self.allowance
                .used
                .fetch_sub(self.bytes - bytes, Ordering::Relaxed);
            self.bytes = bytes;
            Ok(())
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The allowance it is reserved from.
    pub fn allowance(&self) -> &Allowance {
        &self.allowance
    }
}

impl Drop for Reserved {
    fn drop(&mut self) {
        self.allowance.used.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

/// Make room in `buf` for `extra` more bytes, reserving its new capacity in
/// `reserved` before it is allocated: doubled while that fits `max` and the
/// allowance, else exactly what is needed. `reserved` holds at least the
/// buffer's capacity afterwards.
pub(super) fn grow_buffer(
    buf: &mut Vec<u8>,
    reserved: &mut Reserved,
    extra: usize,
    max: usize,
) -> Result<(), Unread> {
    let need = buf.len().checked_add(extra).ok_or(Unread::TooLarge)?;
    if need <= buf.capacity() {
        return Ok(());
    }
    let doubled = buf.capacity().saturating_mul(2).min(max).max(need);
    let target = if doubled > need && reserved.resize(doubled).is_ok() {
        doubled
    } else {
        reserved.resize(need)?;
        need
    };
    buf.reserve_exact(target - buf.len());
    if buf.capacity() > reserved.bytes() {
        reserved.resize(buf.capacity())?;
    }
    Ok(())
}

/// Make room in `items` for one more, reserving the grown capacity in
/// `reserved` before it is allocated.
pub(super) fn grow_items<T>(items: &mut Vec<T>, reserved: &mut Reserved) -> Result<(), Unread> {
    if items.len() < items.capacity() {
        return Ok(());
    }
    let target = items.capacity().saturating_mul(2).max(4);
    reserved.grow((target - items.capacity()) * std::mem::size_of::<T>())?;
    items.reserve_exact(target - items.len());
    let extra = items.capacity().saturating_sub(target);
    if extra > 0 {
        reserved.grow(extra * std::mem::size_of::<T>())?;
    }
    Ok(())
}

/// Insert a copy of `value` into `set` unless it is there, reserving the
/// copy and any growth of the table in `reserved` first. Returns the bytes
/// the copy holds (0 when it was there).
pub(super) fn insert_copy(
    set: &mut std::collections::HashSet<String>,
    value: &str,
    reserved: &mut Reserved,
) -> Result<usize, Unread> {
    if set.contains(value) {
        return Ok(0);
    }
    const SLOT: usize = std::mem::size_of::<String>();
    if set.len() == set.capacity() {
        let before = set.capacity();
        let target = before.saturating_mul(2).max(4);
        reserved.grow(table_bytes(target, SLOT) - table_bytes(before, SLOT))?;
        set.reserve(target - set.len());
        if set.capacity() > target {
            reserved.grow(table_bytes(set.capacity(), SLOT) - table_bytes(target, SLOT))?;
        }
    }
    reserved.grow(value.len())?;
    set.insert(value.to_owned());
    Ok(value.len())
}

/// The bytes a hash table of `capacity` items of `size` bytes holds at most:
/// its buckets (at most about 8/7 of its capacity, rounded up to a power of
/// two) with a control byte each, and a group of control bytes.
pub(super) fn table_bytes(capacity: usize, size: usize) -> usize {
    (capacity.saturating_mul(2) + 2).saturating_mul(size + 1) + 32
}

/// The bytes a hash table of `capacity` items of `size` bytes actually holds,
/// as the standard table lays it out: buckets (a power of two holding 7/8,
/// or one more than the capacity when small) with a control byte each and a
/// group of control bytes. For tests measuring what is kept.
#[cfg(test)]
pub(super) fn table_actual(capacity: usize, size: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    let buckets = if capacity < 8 {
        capacity + 1
    } else {
        capacity * 8 / 7
    };
    buckets * (size + 1) + 16
}

/// A bound on the memory decoding the JSON `line` whole takes, from its
/// structure: the line itself three times (the decoder's scratch while it
/// unescapes a string, its growth, and the owned string), 128 bytes per
/// structural token (a value, its slot in a growing array and a small
/// allocation), and 768 per object (a map's first node). Tested against the
/// actual allocations in [`tests`].
pub(super) fn decoded_bound(line: &[u8]) -> usize {
    let (mut tokens, mut objects) = (0_usize, 0_usize);
    let (mut quoted, mut escaped) = (false, false);
    for &byte in line {
        if quoted {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => {
                quoted = true;
                tokens += 1;
            }
            b'{' => {
                objects += 1;
                tokens += 1;
            }
            b'[' | b',' | b':' => tokens += 1,
            _ => {}
        }
    }
    line.len()
        .saturating_mul(3)
        .saturating_add(tokens.saturating_mul(128))
        .saturating_add(objects.saturating_mul(768))
        .saturating_add(256)
}

/// Where complete lines go: their offset and bytes.
pub(super) type LineSink<'a> = dyn FnMut(u64, &[u8]) -> Result<(), Unread> + 'a;

/// Complete lines out of chunks read in order, carrying a line's start from
/// one chunk to the next within the allowance. A final piece without a
/// newline is not a line yet.
#[derive(Debug)]
pub(super) struct Lines {
    carry: Vec<u8>,
    reserved: Reserved,
    /// The offset of the next byte fed.
    pub next: u64,
    /// The offset just after the last complete line.
    pub complete: u64,
    max_line: usize,
}

impl Lines {
    pub fn new(max_line: usize, allowance: &Allowance) -> Result<Self, Unread> {
        Ok(Self {
            carry: Vec::new(),
            reserved: allowance.reserve(0)?,
            next: 0,
            complete: 0,
            max_line,
        })
    }

    /// Hand every complete line of `chunk` to `line` with its offset, newline
    /// dropped. A line longer than the bound, or one whose carry the
    /// allowance cannot hold, is refused, never partly read.
    pub fn feed(&mut self, chunk: &[u8], line: &mut LineSink<'_>) -> Result<(), Unread> {
        let mut rest = chunk;
        let mut at = self.next;
        while let Some(end) = rest.iter().position(|&byte| byte == b'\n') {
            let piece = &rest[..end];
            if self.carry.len() + piece.len() > self.max_line {
                return Err(Unread::TooLarge);
            }
            let start = self.complete;
            if self.carry.is_empty() {
                line(start, piece)?;
            } else {
                grow_buffer(
                    &mut self.carry,
                    &mut self.reserved,
                    piece.len(),
                    self.max_line,
                )?;
                self.carry.extend_from_slice(piece);
                let whole = std::mem::take(&mut self.carry);
                line(start, &whole)?;
                drop(whole);
                self.reserved.resize(0)?;
            }
            at += end as u64 + 1;
            self.complete = at;
            rest = &rest[end + 1..];
        }
        if self.carry.len() + rest.len() > self.max_line {
            return Err(Unread::TooLarge);
        }
        grow_buffer(
            &mut self.carry,
            &mut self.reserved,
            rest.len(),
            self.max_line,
        )?;
        self.carry.extend_from_slice(rest);
        self.next += chunk.len() as u64;
        Ok(())
    }

    /// Whether everything fed so far ended in complete lines: nothing is
    /// held of a line still being written.
    pub fn finished(&self) -> bool {
        self.complete == self.next
    }

    /// The bytes it actually keeps.
    #[cfg(test)]
    pub fn actual(&self) -> usize {
        self.carry.capacity()
    }
}

/// One complete line starting at `offset`, read forward over as many passes
/// as it needs: the byte before it is a newline (or it is the first), and it
/// ends with one within `max` bytes.
#[derive(Debug)]
pub(super) struct LineRead {
    offset: u64,
    checked: bool,
    buf: Vec<u8>,
    reserved: Reserved,
    max: usize,
}

impl LineRead {
    /// The bytes it actually keeps.
    #[cfg(test)]
    pub fn actual(&self) -> usize {
        self.buf.capacity()
    }

    pub fn new(offset: u64, max: usize, allowance: &Allowance) -> Result<Self, Unread> {
        Ok(Self {
            offset,
            checked: offset == 0,
            buf: Vec::new(),
            reserved: allowance.reserve(0)?,
            max,
        })
    }

    /// Read on within `budget`: `None` when paused, else the line.
    pub fn advance(
        &mut self,
        file: &File,
        budget: &mut Budget<'_>,
    ) -> Result<Option<Vec<u8>>, Unread> {
        if !self.checked {
            let Some(before) = read_chunk(file, self.offset - 1, 1, budget)? else {
                return Ok(None);
            };
            if before != [b'\n'] {
                return Err(Unread::Changed);
            }
            self.checked = true;
        }
        loop {
            let at = self.offset + self.buf.len() as u64;
            let want = ((self.max + 1 - self.buf.len()) as u64).min(line_step(self.buf.len()));
            let Some(chunk) = read_chunk(file, at, want, budget)? else {
                return Ok(None);
            };
            if let Some(end) = chunk.iter().position(|&byte| byte == b'\n') {
                grow_buffer(&mut self.buf, &mut self.reserved, end, self.max)?;
                self.buf.extend_from_slice(&chunk[..end]);
                return Ok(Some(std::mem::take(&mut self.buf)));
            }
            if self.buf.len() + chunk.len() > self.max {
                return Err(Unread::TooLarge);
            }
            grow_buffer(&mut self.buf, &mut self.reserved, chunk.len(), self.max)?;
            self.buf.extend_from_slice(&chunk);
        }
    }
}

/// The complete line that ends exactly at `end` (its newline is the byte
/// before `end`), read backward over as many passes as it needs.
#[derive(Debug)]
pub(super) struct BackRead {
    end: u64,
    /// Where the next backward read ends; `None` until the newline at `end`
    /// was checked.
    cursor: Option<u64>,
    /// The bytes from `cursor` to the newline, in file order.
    buf: Vec<u8>,
    reserved: Reserved,
    max: usize,
}

impl BackRead {
    /// The bytes it actually keeps.
    #[cfg(test)]
    pub fn actual(&self) -> usize {
        self.buf.capacity()
    }

    pub fn new(end: u64, max: usize, allowance: &Allowance) -> Result<Self, Unread> {
        Ok(Self {
            end,
            cursor: None,
            buf: Vec::new(),
            reserved: allowance.reserve(0)?,
            max,
        })
    }

    /// Read on within `budget`: `None` when paused, else the line.
    pub fn advance(
        &mut self,
        file: &File,
        budget: &mut Budget<'_>,
    ) -> Result<Option<Vec<u8>>, Unread> {
        if self.end == 0 {
            return Err(Unread::Changed);
        }
        let mut cursor = match self.cursor {
            Some(cursor) => cursor,
            None => {
                let Some(last) = read_chunk(file, self.end - 1, 1, budget)? else {
                    return Ok(None);
                };
                if last != [b'\n'] {
                    return Err(Unread::Changed);
                }
                self.cursor = Some(self.end - 1);
                self.end - 1
            }
        };
        loop {
            if cursor == 0 {
                return Ok(Some(std::mem::take(&mut self.buf)));
            }
            let want = cursor
                .min((self.max + 1 - self.buf.len()) as u64)
                .min(line_step(self.buf.len()));
            let granted = budget.grant(want);
            if granted == 0 {
                return Ok(None);
            }
            let Some(chunk) = read_chunk(file, cursor - granted, granted, budget)? else {
                return Ok(None);
            };
            if chunk.len() as u64 != granted {
                return Err(Unread::Changed);
            }
            // Earlier bytes go in front, within the reserved capacity.
            if let Some(at) = chunk.iter().rposition(|&byte| byte == b'\n') {
                let head = &chunk[at + 1..];
                grow_buffer(&mut self.buf, &mut self.reserved, head.len(), self.max)?;
                self.buf.splice(0..0, head.iter().copied());
                return Ok(Some(std::mem::take(&mut self.buf)));
            }
            if self.buf.len() + chunk.len() > self.max {
                return Err(Unread::TooLarge);
            }
            grow_buffer(&mut self.buf, &mut self.reserved, chunk.len(), self.max)?;
            self.buf.splice(0..0, chunk.iter().copied());
            cursor -= granted;
            self.cursor = Some(cursor);
        }
    }
}

pub(super) fn generation(meta: &Metadata) -> SegmentGeneration {
    SegmentGeneration {
        device: meta.dev() as i64,
        inode: meta.ino() as i64,
        length: i64::try_from(meta.len()).unwrap_or(i64::MAX),
        mtime_ns: meta.mtime().saturating_mul(1_000_000_000) + meta.mtime_nsec(),
        ctime_ns: meta.ctime().saturating_mul(1_000_000_000) + meta.ctime_nsec(),
    }
}

/// The generation the name holds now, without following an alias.
pub(super) fn stat(path: &Path) -> Result<SegmentGeneration, Unread> {
    let meta = std::fs::symlink_metadata(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => Unread::Missing,
        _ => Unread::Unreadable,
    })?;
    if !meta.file_type().is_file() {
        return Err(Unread::Alias);
    }
    Ok(generation(&meta))
}

/// Open `path` for reading as one regular file with one link, owned by this
/// user, and the generation of what was opened.
pub(super) fn open(path: &Path) -> Result<(File, SegmentGeneration), Unread> {
    let named = std::fs::symlink_metadata(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => Unread::Missing,
        _ => Unread::Unreadable,
    })?;
    if !named.file_type().is_file() {
        return Err(Unread::Alias);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| Unread::Unreadable)?;
    let opened = file.metadata().map_err(|_| Unread::Unreadable)?;
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    if !opened.is_file() || opened.nlink() != 1 || opened.uid() != uid {
        return Err(Unread::Alias);
    }
    if (opened.dev(), opened.ino()) != (named.dev(), named.ino()) {
        return Err(Unread::Changed);
    }
    Ok((file, generation(&opened)))
}

/// The generation an open descriptor describes now.
pub(super) fn generation_of(file: &File) -> Result<SegmentGeneration, Unread> {
    Ok(generation(
        &file.metadata().map_err(|_| Unread::Unreadable)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("{{\"row\":{i:>90}}}")).collect()
    }

    #[test]
    fn lines_come_whole_across_passes_and_every_read_is_within_the_budget() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("rollout.jsonl");
        let rows = rows(100);
        let body = rows.join("\n") + "\n{\"torn\"";
        std::fs::write(&path, &body).unwrap();
        let (file, generation) = open(&path).unwrap();
        let len = u64::try_from(generation.length).unwrap();
        let allowance = Allowance::new(4096);
        let mut lines = Lines::new(1024, &allowance).unwrap();
        let mut seen = Vec::new();
        let mut per_pass = Vec::new();
        let before = READ.with(std::cell::Cell::get);
        // Each pass may read 37 bytes: lines span passes, nothing is read twice.
        while lines.next < len {
            let mut budget = Budget::new(37, Duration::from_secs(60), None);
            while let Some(chunk) =
                read_chunk(&file, lines.next, len - lines.next, &mut budget).unwrap()
            {
                lines
                    .feed(&chunk, &mut |offset, bytes| {
                        seen.push((offset, bytes.to_vec()));
                        Ok(())
                    })
                    .unwrap();
            }
            assert!(budget.spent <= 37);
            per_pass.push(budget.spent);
        }
        assert_eq!(READ.with(std::cell::Cell::get) - before, len);
        assert_eq!(per_pass.iter().sum::<u64>(), len, "no byte read twice");
        assert_eq!(seen.len(), 100);
        let mut at = 0;
        for ((offset, bytes), row) in seen.iter().zip(&rows) {
            assert_eq!((*offset, bytes.as_slice()), (at, row.as_bytes()));
            at += row.len() as u64 + 1;
        }
        assert_eq!(lines.complete, at);
        // The torn tail is still held, within the allowance.
        assert_eq!(allowance.used(), "{\"torn\"".len());
        drop(lines);
        assert_eq!(allowance.used(), 0);
    }

    #[test]
    fn forward_and_backward_line_reads_resume_under_a_tiny_budget() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("rollout.jsonl");
        let rows = rows(5);
        std::fs::write(&path, rows.join("\n") + "\n").unwrap();
        let (file, _) = open(&path).unwrap();
        let allowance = Allowance::new(1 << 20);
        let second = rows[0].len() as u64 + 1;
        let end = second + rows[1].len() as u64 + 1;
        let mut forward = LineRead::new(second, 1024, &allowance).unwrap();
        let mut backward = BackRead::new(end, 1024, &allowance).unwrap();
        let (mut got_forward, mut got_backward) = (None, None);
        let mut passes = 0;
        while got_forward.is_none() || got_backward.is_none() {
            let mut budget = Budget::new(10, Duration::from_secs(60), None);
            if got_forward.is_none() {
                got_forward = forward.advance(&file, &mut budget).unwrap();
            }
            if got_backward.is_none() {
                got_backward = backward.advance(&file, &mut budget).unwrap();
            }
            assert!(budget.spent <= 10);
            passes += 1;
        }
        assert!(passes > 10);
        assert_eq!(got_forward.unwrap(), rows[1].as_bytes());
        assert_eq!(got_backward.unwrap(), rows[1].as_bytes());
        let mut budget = Budget::new(1 << 20, Duration::from_secs(60), None);
        assert_eq!(
            LineRead::new(second + 1, 1024, &allowance)
                .unwrap()
                .advance(&file, &mut budget),
            Err(Unread::Changed)
        );
        assert_eq!(
            BackRead::new(end - 1, 1024, &allowance)
                .unwrap()
                .advance(&file, &mut budget),
            Err(Unread::Changed)
        );
        // A line longer than the bound is refused; so is one the allowance
        // cannot hold.
        assert_eq!(
            LineRead::new(0, 10, &allowance)
                .unwrap()
                .advance(&file, &mut budget),
            Err(Unread::TooLarge)
        );
        let small = Allowance::new(20);
        assert_eq!(
            LineRead::new(0, 1024, &small)
                .unwrap()
                .advance(&file, &mut budget),
            Err(Unread::Memory)
        );
        assert_eq!(small.used(), 0);
        // A spent or cancelled budget grants nothing.
        let mut spent = Budget::new(10, Duration::from_secs(60), None);
        spent.charge(10);
        assert_eq!(spent.grant(1), 0);
        let token = CancelToken::new();
        token.cancel();
        assert_eq!(
            Budget::new(1 << 20, Duration::from_secs(60), Some(&token)).grant(10),
            0
        );
    }

    /// What a partial line keeps is reserved as the buffer's actual
    /// capacity, not only its length, at every step.
    #[test]
    fn partial_lines_reserve_their_actual_capacity() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("rollout.jsonl");
        let long = "y".repeat(10_000);
        std::fs::write(&path, format!("{long}\n{long}\n")).unwrap();
        let (file, generation) = open(&path).unwrap();
        let len = u64::try_from(generation.length).unwrap();
        let allowance = Allowance::new(1 << 20);
        let mut lines = Lines::new(1 << 20, &allowance).unwrap();
        while lines.next < len {
            // Odd-sized pieces, so a growing buffer's capacity passes its length.
            let mut budget = Budget::new(333, Duration::from_secs(60), None);
            let chunk = read_chunk(&file, lines.next, len - lines.next, &mut budget)
                .unwrap()
                .unwrap();
            lines.feed(&chunk, &mut |_, _| Ok(())).unwrap();
            assert!(
                lines.carry.capacity() <= lines.reserved.bytes(),
                "carry capacity {} > reserved {}",
                lines.carry.capacity(),
                lines.reserved.bytes()
            );
            assert_eq!(allowance.used(), lines.reserved.bytes());
        }
        let mut forward = LineRead::new(0, 1 << 20, &allowance).unwrap();
        let mut backward = BackRead::new(len, 1 << 20, &allowance).unwrap();
        let (mut got_forward, mut got_backward) = (None, None);
        while got_forward.is_none() || got_backward.is_none() {
            let mut budget = Budget::new(4_999, Duration::from_secs(60), None);
            if got_forward.is_none() {
                got_forward = forward.advance(&file, &mut budget).unwrap();
            }
            if got_backward.is_none() {
                got_backward = backward.advance(&file, &mut budget).unwrap();
            }
            assert!(
                forward.buf.capacity() <= forward.reserved.bytes(),
                "forward capacity {} > reserved {}",
                forward.buf.capacity(),
                forward.reserved.bytes()
            );
            assert!(
                backward.buf.capacity() <= backward.reserved.bytes(),
                "backward capacity {} > reserved {}",
                backward.buf.capacity(),
                backward.reserved.bytes()
            );
        }
        assert_eq!(got_forward.unwrap().len(), 10_000);
        assert_eq!(got_backward.unwrap().len(), 10_000);
    }

    #[test]
    fn an_alias_or_a_second_link_is_not_opened() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("a.jsonl");
        std::fs::write(&path, b"x\n").unwrap();
        let link = temp.path().join("b.jsonl");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(open(&link).map(|_| ()), Err(Unread::Alias));
        let hard = temp.path().join("c.jsonl");
        std::fs::hard_link(&path, &hard).unwrap();
        assert_eq!(open(&path).map(|_| ()), Err(Unread::Alias));
        assert_eq!(
            open(&temp.path().join("none")).map(|_| ()),
            Err(Unread::Missing)
        );
    }
}
