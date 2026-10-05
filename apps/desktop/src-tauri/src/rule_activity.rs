//! Recorded rule activity, read on demand from the default local rulebook.
//!
//! One read is one bounded snapshot of `~/.config/memhub-plugin/rulebook`'s
//! ledger over the fixed trailing 14 × 24 hours. The window is anchored once,
//! by this service, when a read is admitted; the view names nothing but its
//! own identifier for the read. No root, path, time range or filter crosses
//! the boundary, and nothing here runs on a timer, at startup, on focus or on
//! an event: only an explicit read reads.
//!
//! The source is chosen once, from the native home startup already selected
//! and validated. Constructing the service touches nothing on disk. Fixture
//! mode has no native home and so no source: it answers `not_configured`, and
//! never falls back to the user's own home.
//!
//! One read runs at a time. A second read, under any identifier, is `busy`
//! and reads nothing; the running read is neither shared nor replaced. The
//! slot is taken before any source access, released only when the reading
//! worker has returned (on every path, a panic included), and never queued:
//! admission happens before a worker is spawned, so a refused read costs no
//! worker at all. The read holds no database lock and needs none.
//!
//! Cancellation is cooperative, like the transcript reads': a cancel that
//! arrives before its read is remembered (a bounded number of them), one
//! that arrives during it cancels that read alone, and a cancel never frees
//! the slot early — a read blocked inside an OS call still holds it until it
//! returns. Closing refuses new reads, cancels the running one and waits a
//! bounded time for it; it cannot preempt a blocked OS call either. Nothing
//! is persisted, cached or logged, and an identifier never reaches a file or
//! an error.
use crate::rule_activity_dto::{RuleActivityBusy, RuleActivityInterruption, RuleActivityResult};
use jiff::{SignedDuration, Timestamp};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use xt_ingest::native::readers_cli::CancelToken;
use xt_rulebook::activity::{ActivityRead, ActivityWindow, ReadLimits};

/// The fixed trailing window every read covers: 14 × 24 hours, not a
/// calendar period.
pub const WINDOW: SignedDuration = SignedDuration::from_hours(14 * 24);

/// How long closing waits for the running read to end after cancelling it,
/// the same bound the transcript reads have.
pub const SHUTDOWN_BOUND: Duration = crate::transcript_reads::SHUTDOWN_BOUND;

/// How many cancellations are remembered for reads that have not started.
const REMEMBERED_CANCELS: usize = 64;

/// The longest identifier a view may name a read with.
const MAX_ID: usize = 128;

type Reader = dyn Fn(&Path, &ReadLimits, Option<ActivityWindow>, &dyn Fn() -> bool) -> ActivityRead
    + Send
    + Sync;
type Clock = dyn Fn() -> Timestamp + Send + Sync;

pub struct RuleActivityService {
    /// The default source root, or none when this process has no source.
    root: Option<PathBuf>,
    reader: Arc<Reader>,
    clock: Arc<Clock>,
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    /// Signalled when the running read ends, so closing can wait for it.
    ended: Condvar,
}

#[derive(Default)]
struct State {
    /// The one running read: its identifier and cancel token.
    active: Option<(String, CancelToken)>,
    /// Cancellations that arrived before the read they name. Both halves are
    /// kept in step so the set never outgrows the queue.
    cancelled: HashSet<String>,
    order: VecDeque<String>,
    /// Set once by close: every later read is refused.
    closed: bool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A read that panicked never holds this lock (it is taken only for
        // bookkeeping), and refusing every later read would turn one failure
        // into a page that can never read again.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// An admitted read, ready to run on a worker. It owns the slot: dropping it,
/// run or not, releases the slot.
pub struct AdmittedRead {
    slot: Slot,
    read_id: String,
    root: PathBuf,
    window: ActivityWindow,
    reader: Arc<Reader>,
}

/// The slot of the one running read. Dropping it frees the slot, on every
/// path including a panic, and wakes a waiting close.
struct Slot {
    shared: Arc<Shared>,
    token: CancelToken,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.active = None;
        drop(state);
        self.shared.ended.notify_all();
    }
}

fn valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

impl RuleActivityService {
    /// The service over the default source under the native `home` startup
    /// selected, or over no source when there is none (fixture mode). Only
    /// the path is computed: nothing is read, checked or created.
    pub fn new(home: Option<&Path>) -> Self {
        Self::with_parts(
            home.map(xt_rulebook::activity::default_root),
            Arc::new(xt_rulebook::activity::read_activity),
            Arc::new(Timestamp::now),
        )
    }

    fn with_parts(root: Option<PathBuf>, reader: Arc<Reader>, clock: Arc<Clock>) -> Self {
        Self {
            root,
            reader,
            clock,
            shared: Arc::default(),
        }
    }

    /// Read on the calling thread: admission, then the read itself.
    pub fn read(&self, read_id: &str) -> RuleActivityResult {
        match self.admit(read_id) {
            Ok(admitted) => admitted.run(),
            Err(result) => result,
        }
    }

    /// Take the one slot for `read_id` and anchor its window, or answer at
    /// once without reading anything.
    pub fn admit(&self, read_id: &str) -> Result<AdmittedRead, RuleActivityResult> {
        if !valid(read_id) {
            return Err(RuleActivityResult::InvalidReadId);
        }
        let mut state = self.shared.lock();
        if state.closed {
            return Err(RuleActivityResult::Closed);
        }
        // The slot is checked before a remembered cancel is consumed: a
        // refused read is not an answered one, so its cancel is kept for the
        // attempt that is admitted.
        if let Some((active, _)) = &state.active {
            let reason = if active == read_id {
                RuleActivityBusy::DuplicateRead
            } else {
                RuleActivityBusy::AnotherRead
            };
            return Err(RuleActivityResult::Busy { reason });
        }
        if state.cancelled.remove(read_id) {
            state.order.retain(|remembered| remembered != read_id);
            return Err(RuleActivityResult::Interrupted {
                reason: RuleActivityInterruption::Cancelled,
            });
        }
        let Some(root) = self.root.clone() else {
            return Err(RuleActivityResult::not_configured());
        };
        let end = (self.clock)();
        let window = end
            .checked_sub(WINDOW)
            .ok()
            .and_then(|start| ActivityWindow::new(start, end))
            .ok_or(RuleActivityResult::Failed)?;
        let token = CancelToken::new();
        state.active = Some((read_id.to_owned(), token.clone()));
        drop(state);
        Ok(AdmittedRead {
            slot: Slot {
                shared: Arc::clone(&self.shared),
                token,
            },
            read_id: read_id.to_owned(),
            root,
            window,
            reader: Arc::clone(&self.reader),
        })
    }

    /// Cancel the read this view named, whether or not it has started. An
    /// invalid name is not remembered: it could not have started a read.
    pub fn cancel(&self, read_id: &str) {
        if !valid(read_id) {
            return;
        }
        let mut state = self.shared.lock();
        if let Some((active, token)) = &state.active
            && active == read_id
        {
            token.cancel();
            return;
        }
        if state.cancelled.insert(read_id.to_owned()) {
            state.order.push_back(read_id.to_owned());
            while state.order.len() > REMEMBERED_CANCELS {
                if let Some(forgotten) = state.order.pop_front() {
                    state.cancelled.remove(&forgotten);
                }
            }
        }
    }

    /// Refuse every later read, cancel the running one, and wait — at most
    /// [`SHUTDOWN_BOUND`] — for it to return. Called when the app exits,
    /// before the index and the database close. Returns whether no read was
    /// still running when it returned.
    pub fn close(&self) -> bool {
        self.close_within(SHUTDOWN_BOUND)
    }

    /// [`Self::close`] with an explicit bound.
    pub fn close_within(&self, bound: Duration) -> bool {
        let deadline = Instant::now() + bound;
        let mut state = self.shared.lock();
        state.closed = true;
        if let Some((_, token)) = &state.active {
            token.cancel();
        }
        while state.active.is_some() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            state = self
                .shared
                .ended
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        true
    }
}

impl AdmittedRead {
    /// Read the source and convert the snapshot. Blocks for as long as the
    /// read's own bounds allow; run it on a blocking worker.
    pub fn run(self) -> RuleActivityResult {
        let token = self.slot.token.clone();
        let cancelled = move || token.is_cancelled();
        let read = (self.reader)(
            &self.root,
            &ReadLimits::default(),
            Some(self.window),
            &cancelled,
        );
        let result = RuleActivityResult::from_read(&self.read_id, self.window, read);
        // One last look after the conversion: a read cancelled or closed
        // while it was converted returns nothing from it.
        if self.slot.shared.lock().closed {
            return RuleActivityResult::Closed;
        }
        if cancelled() {
            return RuleActivityResult::Interrupted {
                reason: RuleActivityInterruption::Cancelled,
            };
        }
        result
    }
}

/// Admit on the caller, then read on the blocking pool. A refused read
/// answers at once and spawns nothing, so no refused request ever waits in
/// the pool; a worker that panics is `failed`, and its slot is released as
/// it unwinds.
pub async fn read_on_worker(service: &RuleActivityService, read_id: &str) -> RuleActivityResult {
    match service.admit(read_id) {
        Ok(admitted) => tauri::async_runtime::spawn_blocking(move || admitted.run())
            .await
            .unwrap_or(RuleActivityResult::Failed),
        Err(result) => result,
    }
}

#[cfg(test)]
mod tests;
