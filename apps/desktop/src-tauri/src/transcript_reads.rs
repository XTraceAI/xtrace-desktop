//! The cancel tokens of the transcript reads running now — one per open.
//!
//! Opening a session reads its whole source into memory. A reader who opens a
//! session and immediately navigates away, or opens a different one, should
//! not leave that read running: the ceilings bound how much it may read, not
//! how long it may take, and a detail view is interactive.
//!
//! The view names each open with an identifier of its own and cancels **that**
//! identifier when the open is replaced or abandoned. Two properties make that
//! safe whatever order the two calls arrive in:
//!
//! - A cancel that arrives **after** its read started cancels it, because the
//!   token is registered under that name while the read runs.
//! - A cancel that arrives **before** its read started is remembered, so the
//!   read begins already cancelled and returns with no content. Without that,
//!   a cancel racing an open would silently do nothing — the window is small
//!   and a command crossing a process boundary is exactly where it is lost.
//!
//! Everything here is bounded: one entry per running read, removed by the read
//! itself on every path, a fixed cap on how many may run at once, and a fixed
//! number of remembered cancellations. Nothing is persisted and nothing is
//! logged; an identifier is an opaque name the view chose, and it never
//! reaches a file, a query or an error.
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use xt_ingest::native::readers_cli::CancelToken;

/// How long shutdown waits for the reads it cancelled to finish. A cancelled
/// read ends at its next check: a pinned reader's supervisor polls every 10 ms
/// and kills and reaps the reader's group before it returns, and an
/// interpreter probe is killed at once. The bound is for a read that is
/// somewhere slower — hashing the bundle, or waiting for a reader's output to
/// close after its group was killed — so quitting can never hang on one.
pub const SHUTDOWN_BOUND: Duration = Duration::from_secs(3);

/// The most reads that may run at once. A detail view opens one session at a
/// time; this is the ceiling on a caller that does not.
pub const MAX_ACTIVE_READS: usize = 8;

/// How many cancellations are remembered for reads that have not started yet.
/// Older ones are forgotten: the read they named either started long ago or is
/// never going to.
const REMEMBERED_CANCELS: usize = 64;

/// The longest identifier a view may name a read with. An opaque name is
/// short; a longer one is a value being used as a key.
const MAX_ID: usize = 128;

#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The identifier is empty, too long, or not printable.
    InvalidId,
    /// A read is already running under this identifier.
    Duplicate,
    /// Too many reads are running at once.
    TooMany,
    /// The app is shutting down; no read starts.
    Closed,
}

#[derive(Default)]
struct Reads {
    active: HashMap<String, CancelToken>,
    /// Cancellations that arrived before the read they name. Both halves are
    /// kept in step so the set never outgrows the queue.
    cancelled: HashSet<String>,
    order: VecDeque<String>,
    /// Set once by shutdown: every later read is refused.
    closed: bool,
}

#[derive(Default)]
pub struct TranscriptReads {
    reads: Mutex<Reads>,
    /// Signalled whenever a read ends, so shutdown can wait for the last one.
    ended: Condvar,
}

/// A running read's registration. Dropping it removes the entry, on every
/// path including a panic, so a read can never leave its name behind.
pub struct ReadGuard<'a> {
    reads: &'a TranscriptReads,
    id: String,
    token: CancelToken,
}

impl ReadGuard<'_> {
    pub fn token(&self) -> &CancelToken {
        &self.token
    }
}

impl Drop for ReadGuard<'_> {
    fn drop(&mut self) {
        let mut reads = self.reads.lock();
        reads.active.remove(&self.id);
        drop(reads);
        // The read has returned by now: whatever it started has been killed
        // and reaped by its own supervisor, which is what shutdown waits for.
        self.reads.ended.notify_all();
    }
}

fn valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

impl TranscriptReads {
    fn lock(&self) -> std::sync::MutexGuard<'_, Reads> {
        // A poisoned lock means a read panicked while holding it; the map is
        // still a map of names, and refusing every later read would turn one
        // failure into a screen that can never open a session again.
        self.reads
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Register a read under the view's own name for it.
    ///
    /// The returned guard holds the token the read must observe. If this name
    /// was already cancelled, that token is returned **already cancelled**, so
    /// the read ends at its first check with no content.
    pub fn begin(&self, id: &str) -> Result<ReadGuard<'_>, ReadError> {
        if !valid(id) {
            return Err(ReadError::InvalidId);
        }
        let mut reads = self.lock();
        if reads.closed {
            return Err(ReadError::Closed);
        }
        if reads.active.contains_key(id) {
            return Err(ReadError::Duplicate);
        }
        if reads.active.len() >= MAX_ACTIVE_READS {
            return Err(ReadError::TooMany);
        }
        let token = CancelToken::new();
        if reads.cancelled.remove(id) {
            reads.order.retain(|remembered| remembered != id);
            token.cancel();
        }
        reads.active.insert(id.to_owned(), token.clone());
        drop(reads);
        Ok(ReadGuard {
            reads: self,
            id: id.to_owned(),
            token,
        })
    }

    /// Cancel the read this view named, whether or not it has started.
    ///
    /// Cancelling something that never runs costs one remembered name, and a
    /// bounded number of those are kept. An invalid name is not remembered: it
    /// could not have started a read either.
    pub fn cancel(&self, id: &str) {
        if !valid(id) {
            return;
        }
        let mut reads = self.lock();
        if let Some(token) = reads.active.get(id) {
            token.cancel();
            return;
        }
        if reads.cancelled.insert(id.to_owned()) {
            reads.order.push_back(id.to_owned());
            while reads.order.len() > REMEMBERED_CANCELS {
                if let Some(forgotten) = reads.order.pop_front() {
                    reads.cancelled.remove(&forgotten);
                }
            }
        }
    }

    /// Cancel every read running now, refuse any that would start, and wait
    /// — at most [`SHUTDOWN_BOUND`] — for the cancelled reads to end. Called
    /// when the app exits, before the index and the database close.
    ///
    /// Waiting is the point. A pinned reader's only reaper is the supervisor
    /// running inside the read; a cancel only asks it to stop. The app exits
    /// the process as soon as its exit handler returns, which would end the
    /// supervisor before it had killed the reader's group, and leave the
    /// reader running after the app was gone. A read's registration is
    /// released only after the read has returned, so an empty registry means
    /// every reader it started has been killed and reaped.
    ///
    /// The lock is released while waiting, so a read that is ending can take
    /// it to remove itself. Returns whether every read ended within the bound.
    pub fn shutdown(&self) -> bool {
        self.shutdown_within(SHUTDOWN_BOUND)
    }

    /// [`Self::shutdown`] with an explicit bound.
    pub fn shutdown_within(&self, bound: Duration) -> bool {
        let deadline = Instant::now() + bound;
        let mut reads = self.lock();
        reads.closed = true;
        for token in reads.active.values() {
            token.cancel();
        }
        while !reads.active.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            reads = self
                .ended
                .wait_timeout(reads, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        true
    }

    #[cfg(test)]
    fn active(&self) -> usize {
        self.lock().active.len()
    }

    #[cfg(test)]
    fn remembered(&self) -> usize {
        let reads = self.lock();
        assert_eq!(reads.cancelled.len(), reads.order.len());
        reads.order.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_that_lands_during_a_read_cancels_it() {
        let reads = TranscriptReads::default();
        let guard = reads.begin("open-1").unwrap();
        assert!(!guard.token().is_cancelled());
        reads.cancel("open-1");
        assert!(guard.token().is_cancelled());
    }

    #[test]
    fn a_cancel_that_lands_before_its_read_still_cancels_it() {
        // The view abandoned the open while the command was still crossing.
        let reads = TranscriptReads::default();
        reads.cancel("open-2");
        assert_eq!(reads.remembered(), 1);
        let guard = reads.begin("open-2").unwrap();
        assert!(guard.token().is_cancelled());
        // Remembered exactly once: the read it named has now started.
        assert_eq!(reads.remembered(), 0);
    }

    #[test]
    fn a_cancel_for_one_read_leaves_the_others_running() {
        let reads = TranscriptReads::default();
        let first = reads.begin("open-a").unwrap();
        let second = reads.begin("open-b").unwrap();
        reads.cancel("open-b");
        assert!(!first.token().is_cancelled());
        assert!(second.token().is_cancelled());
    }

    #[test]
    fn a_finished_read_leaves_no_entry_and_its_name_is_reusable() {
        let reads = TranscriptReads::default();
        {
            let _guard = reads.begin("open-3").unwrap();
            assert_eq!(reads.active(), 1);
            assert_eq!(reads.begin("open-3").err(), Some(ReadError::Duplicate));
        }
        assert_eq!(reads.active(), 0);
        // A name whose read completed is ordinary again: a retry under it is
        // not born cancelled, because nothing cancelled it.
        assert!(!reads.begin("open-3").unwrap().token().is_cancelled());
    }

    #[test]
    fn a_panicking_read_still_releases_its_name() {
        let reads = TranscriptReads::default();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = reads.begin("open-4").unwrap();
            panic!("a read failed");
        }));
        assert!(caught.is_err());
        assert_eq!(reads.active(), 0);
        assert!(reads.begin("open-4").is_ok());
    }

    #[test]
    fn reads_running_at_once_are_capped() {
        let reads = TranscriptReads::default();
        let held: Vec<_> = (0..MAX_ACTIVE_READS)
            .map(|index| reads.begin(&format!("open-{index}")).unwrap())
            .collect();
        assert_eq!(reads.begin("one-too-many").err(), Some(ReadError::TooMany));
        drop(held);
        assert!(reads.begin("one-too-many").is_ok());
    }

    #[test]
    fn remembered_cancellations_are_bounded_and_oldest_first() {
        let reads = TranscriptReads::default();
        for index in 0..REMEMBERED_CANCELS + 10 {
            reads.cancel(&format!("never-{index}"));
        }
        assert_eq!(reads.remembered(), REMEMBERED_CANCELS);
        // The oldest were forgotten, so their reads would start normally.
        assert!(!reads.begin("never-0").unwrap().token().is_cancelled());
        // The newest is still remembered.
        let last = format!("never-{}", REMEMBERED_CANCELS + 9);
        assert!(reads.begin(&last).unwrap().token().is_cancelled());
    }

    #[test]
    fn shutdown_cancels_every_running_read_and_refuses_new_ones() {
        let reads = TranscriptReads::default();
        assert!(reads.shutdown_within(Duration::ZERO));
        assert_eq!(reads.begin("open-c").err(), Some(ReadError::Closed));
    }

    /// Shutdown returns only once every cancelled read has ended — with the
    /// lock released meanwhile, or the reads could never remove themselves.
    #[test]
    fn shutdown_waits_for_cancelled_reads_to_end() {
        let reads = TranscriptReads::default();
        let ended = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let (started, all_started) = std::sync::mpsc::channel();
            for name in ["open-a", "open-b"] {
                let (reads, ended, started) = (&reads, &ended, started.clone());
                scope.spawn(move || {
                    let guard = reads.begin(name).unwrap();
                    started.send(()).unwrap();
                    while !guard.token().is_cancelled() {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    // Cleanup that takes a while after the cancel lands.
                    std::thread::sleep(Duration::from_millis(100));
                    ended.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                });
            }
            all_started.recv().unwrap();
            all_started.recv().unwrap();
            assert!(reads.shutdown_within(Duration::from_secs(10)));
            // Both had finished their cleanup before shutdown returned.
            assert_eq!(ended.load(std::sync::atomic::Ordering::SeqCst), 2);
            assert_eq!(reads.active(), 0);
            assert_eq!(reads.begin("open-a").err(), Some(ReadError::Closed));
        });
    }

    /// A read that does not end is waited for no longer than the bound.
    #[test]
    fn shutdown_gives_up_on_a_read_that_does_not_end_within_its_bound() {
        let reads = TranscriptReads::default();
        let guard = reads.begin("stuck").unwrap();
        let started = Instant::now();
        assert!(!reads.shutdown_within(Duration::from_millis(100)));
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(guard.token().is_cancelled());
    }

    #[test]
    fn repeating_a_cancel_remembers_one_name() {
        let reads = TranscriptReads::default();
        for _ in 0..5 {
            reads.cancel("open-5");
        }
        assert_eq!(reads.remembered(), 1);
    }

    #[test]
    fn a_name_that_could_not_have_started_a_read_is_refused_and_not_remembered() {
        let reads = TranscriptReads::default();
        for id in ["", "a/b", "../etc", "open 6", &"x".repeat(MAX_ID + 1)] {
            assert_eq!(reads.begin(id).err(), Some(ReadError::InvalidId), "{id}");
            reads.cancel(id);
        }
        assert_eq!(reads.remembered(), 0);
    }
}
