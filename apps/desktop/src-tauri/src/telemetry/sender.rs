//! The Rust sender: a bounded queue of eligible events, batch
//! delivery with bounded retries, and the local ledger of what was tried.
//!
//! The gate is checked when an event is made, again when a batch is taken
//! and once more immediately before every request, including retries. Turning
//! telemetry off drops everything queued or waiting to retry, cancels the
//! request in flight as far as possible and starts a new epoch: a request
//! that returns afterwards never queues anything again, and nothing dropped
//! is ever replayed. An event made while telemetry may not send is dropped,
//! never queued or recorded.
use super::consent::Gate;
use super::events::{AppVersion, CommonProperties, DetectedHost, OsVersion, TelemetryEvent};
use super::heartbeat;
use super::posthog::{self, PostHogConfig};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;
use xt_store::telemetry::DeliveryStatus;

/// Events waiting to be sent or retried; a new event beyond this is failed.
pub const QUEUE_CAPACITY: usize = 50;
/// Events in one request.
pub const BATCH_MAX: usize = 20;
/// Requests per event before it is failed for good.
pub const MAX_ATTEMPTS: u32 = 4;
/// The wait before the first retry; each later retry waits four times longer.
pub const FIRST_RETRY_MS: i64 = 30_000;

/// The wait after `attempts` failed requests: 30 s, 2 min, 8 min.
pub fn retry_delay_ms(attempts: u32) -> i64 {
    FIRST_RETRY_MS * 4_i64.pow(attempts.saturating_sub(1).min(8))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// No accepted response: refused, unreachable or timed out.
    Failed,
    /// Stopped because telemetry was turned off or the app is quitting.
    Cancelled,
}

/// Delivers one batch body. `Ok` only for an accepted (2xx) response.
pub trait Transport: Send + Sync {
    fn send(&self, body: &[u8], cancel: &Cancel) -> Result<(), SendError>;
}

/// The cancel of one request; a transport checks it while it waits.
#[derive(Debug, Default)]
pub struct Cancel(AtomicBool);
impl Cancel {
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Wall time and the local calendar day.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
    /// `YYYY-MM-DD` in the local time zone.
    fn local_day(&self, ms: i64) -> String;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        jiff::Timestamp::now().as_millisecond()
    }
    fn local_day(&self, ms: i64) -> String {
        jiff::Timestamp::from_millisecond(ms)
            .map(|t| t.to_zoned(jiff::tz::TimeZone::system()).date().to_string())
            .unwrap_or_default()
    }
}

/// What the sender reads and writes in the app's store. Every method fails
/// closed: an unreadable store sends nothing.
pub trait TelemetryStore: Send + Sync {
    fn hosts_detected(&self) -> Option<Vec<DetectedHost>>;
    fn distinct_id(&self) -> Option<String>;
    fn record_pending(&self, at_ms: i64, event: &str, properties_json: &str) -> Option<i64>;
    fn mark(&self, id: i64, status: DeliveryStatus);
    fn fail_pending(&self);
    fn daily_active_day(&self) -> Option<Option<String>>;
    fn set_daily_active_day(&self, day: &str) -> bool;
    fn hourly_active_at(&self) -> Option<Option<i64>>;
    fn set_hourly_active_at(&self, at_ms: i64) -> bool;
}

struct Pending {
    ledger_id: i64,
    event: Value,
    attempts: u32,
    next_at_ms: i64,
}

/// A taken batch: its events, the epoch it was taken in and its cancel.
pub(crate) struct Batch {
    items: Vec<Pending>,
    epoch: u64,
    cancel: Arc<Cancel>,
}

#[derive(Default)]
struct Inner {
    queue: VecDeque<Pending>,
    /// Advanced each time the gate closes; a batch from an older epoch is
    /// never queued again.
    epoch: u64,
    in_flight: Option<Arc<Cancel>>,
    opened: bool,
    activity: bool,
    stop: bool,
}

pub(crate) struct Shared {
    inner: Mutex<Inner>,
    wake: Condvar,
    gate: Arc<Gate>,
    transport: Arc<dyn Transport>,
    store: Arc<dyn TelemetryStore>,
    clock: Arc<dyn Clock>,
    config: PostHogConfig,
    app_version: AppVersion,
    os_version: OsVersion,
}

pub struct Sender {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Sender {
    /// Fails anything an earlier run left pending (never sent again), then
    /// starts the delivery thread.
    pub fn start(
        gate: Arc<Gate>,
        config: PostHogConfig,
        transport: Arc<dyn Transport>,
        store: Arc<dyn TelemetryStore>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let shared = Arc::new(Shared::new(
            gate,
            config,
            transport,
            store,
            clock,
            OsVersion::current(),
        ));
        shared.store.fail_pending();
        let worker = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("telemetry".into())
                .spawn(move || shared.run())
                .ok()
        };
        Self {
            shared,
            worker: Mutex::new(worker),
        }
    }

    /// The app started: `app_opened`, then `daily_active` when due.
    pub fn opened(&self) {
        self.shared.signal(|inner| inner.opened = true);
    }

    /// The window or popover was used: the heartbeats that are due.
    pub fn activity(&self) {
        self.shared.signal(|inner| inner.activity = true);
    }

    /// The gate closed: drop queued and retrying work, cancel the request in
    /// flight.
    pub fn close(&self) {
        self.shared.close();
    }

    /// Stops the thread, cancelling its request, before the store closes.
    pub fn shutdown(&self) {
        self.shared.signal(|inner| inner.stop = true);
        self.shared.close();
        let worker = lock(&self.worker).take();
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub(crate) fn new(
        gate: Arc<Gate>,
        config: PostHogConfig,
        transport: Arc<dyn Transport>,
        store: Arc<dyn TelemetryStore>,
        clock: Arc<dyn Clock>,
        os_version: OsVersion,
    ) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            wake: Condvar::new(),
            gate,
            transport,
            store,
            clock,
            config,
            app_version: AppVersion::current(),
            os_version,
        }
    }

    fn signal(&self, set: impl FnOnce(&mut Inner)) {
        set(&mut lock(&self.inner));
        self.wake.notify_all();
    }

    fn run(&self) {
        loop {
            let (opened, activity) = {
                let mut inner = lock(&self.inner);
                loop {
                    if inner.stop {
                        return;
                    }
                    if inner.opened || inner.activity {
                        break;
                    }
                    let now = self.clock.now_ms();
                    let wait = match inner.queue.iter().map(|p| p.next_at_ms).min() {
                        Some(due) if due <= now => break,
                        Some(due) => Duration::from_millis((due - now) as u64),
                        None => Duration::from_secs(3600),
                    };
                    inner = self
                        .wake
                        .wait_timeout(inner, wait)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
                (
                    std::mem::take(&mut inner.opened),
                    std::mem::take(&mut inner.activity),
                )
            };
            if opened {
                self.on_opened();
            }
            if activity {
                self.on_activity();
            }
            while self.deliver_once() {}
        }
    }

    pub(crate) fn on_opened(&self) {
        self.emit(TelemetryEvent::AppOpened);
        self.daily();
    }

    pub(crate) fn on_activity(&self) {
        self.daily();
        let now = self.clock.now_ms();
        // Saved, so a restart within the hour does not send another.
        let Some(last) = self.store.hourly_active_at() else {
            return;
        };
        if heartbeat::hourly_due(last, now) && self.emit(TelemetryEvent::HourlyActive) {
            self.store.set_hourly_active_at(now);
        }
    }

    fn daily(&self) {
        let today = self.clock.local_day(self.clock.now_ms());
        let Some(last) = self.store.daily_active_day() else {
            return;
        };
        if heartbeat::daily_due(last.as_deref(), &today) && self.emit(TelemetryEvent::DailyActive) {
            self.store.set_daily_active_day(&today);
        }
    }

    /// Queues one event when the gate allows it; returns whether it was
    /// queued. A dropped event leaves no trace.
    pub(crate) fn emit(&self, event: TelemetryEvent) -> bool {
        if !event.sent_by_this_build() || !self.gate.allows() || lock(&self.inner).stop {
            return false;
        }
        let (Some(hosts_detected), Some(distinct_id)) =
            (self.store.hosts_detected(), self.store.distinct_id())
        else {
            return false;
        };
        let properties = event.properties(&CommonProperties {
            app_version: self.app_version.clone(),
            os_version: self.os_version.clone(),
            hosts_detected,
        });
        let now = self.clock.now_ms();
        let Ok(properties_json) = serde_json::to_string(&properties) else {
            return false;
        };
        let Some(ledger_id) = self
            .store
            .record_pending(now, event.name(), &properties_json)
        else {
            return false;
        };
        let object = posthog::event_object(event.name(), &distinct_id, now, &properties);
        {
            let mut inner = lock(&self.inner);
            // Turned off, or the app began quitting, while the event was
            // being made: it is not queued.
            if self.gate.allows() && !inner.stop && inner.queue.len() < QUEUE_CAPACITY {
                inner.queue.push_back(Pending {
                    ledger_id,
                    event: object,
                    attempts: 0,
                    next_at_ms: now,
                });
                drop(inner);
                self.wake.notify_all();
                return true;
            }
        }
        self.store.mark(ledger_id, DeliveryStatus::Failed);
        false
    }

    /// Drops all queued and retrying work and cancels the request in flight.
    pub(crate) fn close(&self) {
        let dropped: Vec<_> = {
            let mut inner = lock(&self.inner);
            inner.epoch += 1;
            if let Some(cancel) = inner.in_flight.take() {
                cancel.cancel();
            }
            inner.queue.drain(..).map(|p| p.ledger_id).collect()
        };
        for id in dropped {
            self.store.mark(id, DeliveryStatus::Failed);
        }
    }

    /// The due events, oldest first, when the gate allows sending.
    pub(crate) fn take_batch(&self, now: i64) -> Option<Batch> {
        if !self.gate.allows() {
            self.close();
            return None;
        }
        let mut inner = lock(&self.inner);
        if inner.stop {
            return None;
        }
        let mut items = Vec::new();
        let mut kept = VecDeque::new();
        for pending in inner.queue.drain(..) {
            if pending.next_at_ms <= now && items.len() < BATCH_MAX {
                items.push(pending);
            } else {
                kept.push_back(pending);
            }
        }
        inner.queue = kept;
        if items.is_empty() {
            return None;
        }
        let cancel = Arc::new(Cancel::default());
        inner.in_flight = Some(cancel.clone());
        Some(Batch {
            items,
            epoch: inner.epoch,
            cancel,
        })
    }

    /// Sends a taken batch, checking the gate immediately before.
    pub(crate) fn send(&self, batch: &Batch) -> Result<(), SendError> {
        let (current, stopping) = {
            let inner = lock(&self.inner);
            (inner.epoch == batch.epoch, inner.stop)
        };
        if !current || stopping || !self.gate.allows() || batch.cancel.is_cancelled() {
            return Err(SendError::Cancelled);
        }
        let events: Vec<_> = batch.items.iter().map(|p| p.event.clone()).collect();
        self.transport
            .send(&self.config.batch_body(&events), &batch.cancel)
    }

    /// Records a batch's outcome. Only an accepted request marks events sent;
    /// a failure is retried with backoff while its epoch is current and the
    /// gate still allows sending, and failed for good otherwise.
    pub(crate) fn finish(&self, batch: Batch, result: Result<(), SendError>, now: i64) {
        let mut marks = Vec::new();
        {
            let mut inner = lock(&self.inner);
            if inner
                .in_flight
                .as_ref()
                .is_some_and(|c| Arc::ptr_eq(c, &batch.cancel))
            {
                inner.in_flight = None;
            }
            let retry = result == Err(SendError::Failed)
                && batch.epoch == inner.epoch
                && !inner.stop
                && self.gate.allows();
            for mut pending in batch.items.into_iter().rev() {
                if result.is_ok() {
                    marks.push((pending.ledger_id, DeliveryStatus::Sent));
                    continue;
                }
                pending.attempts += 1;
                if retry && pending.attempts < MAX_ATTEMPTS {
                    pending.next_at_ms = now + retry_delay_ms(pending.attempts);
                    inner.queue.push_front(pending);
                } else {
                    marks.push((pending.ledger_id, DeliveryStatus::Failed));
                }
            }
        }
        for (id, status) in marks.into_iter().rev() {
            self.store.mark(id, status);
        }
    }

    /// Takes, sends and finishes one batch; false when none was due.
    pub(crate) fn deliver_once(&self) -> bool {
        let Some(batch) = self.take_batch(self.clock.now_ms()) else {
            return false;
        };
        let result = self.send(&batch);
        self.finish(batch, result, self.clock.now_ms());
        true
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::consent::Consent;
    use super::*;
    use std::sync::atomic::AtomicI64;
    use xt_store::Store;

    /// The app store's telemetry calls over a test database.
    pub(crate) struct TestStore(pub Mutex<Store>);
    impl TestStore {
        pub(crate) fn new(store: Store) -> Arc<Self> {
            Arc::new(Self(Mutex::new(store)))
        }
        pub(crate) fn ledger(&self) -> Vec<(String, DeliveryStatus)> {
            let mut rows = lock(&self.0).telemetry_ledger(1000).unwrap();
            rows.reverse();
            rows.into_iter().map(|r| (r.event, r.status)).collect()
        }
    }
    impl TelemetryStore for TestStore {
        fn hosts_detected(&self) -> Option<Vec<DetectedHost>> {
            Some(
                lock(&self.0)
                    .indexed_hosts()
                    .ok()?
                    .into_iter()
                    .filter_map(DetectedHost::from_store)
                    .collect(),
            )
        }
        fn distinct_id(&self) -> Option<String> {
            lock(&self.0)
                .telemetry_distinct_id(|| "00000000-0000-4000-8000-00000000test".into())
                .ok()
        }
        fn record_pending(&self, at_ms: i64, event: &str, properties_json: &str) -> Option<i64> {
            lock(&self.0)
                .telemetry_record_pending(at_ms, event, properties_json)
                .ok()
        }
        fn mark(&self, id: i64, status: DeliveryStatus) {
            lock(&self.0).telemetry_mark(id, status).unwrap();
        }
        fn fail_pending(&self) {
            lock(&self.0).telemetry_fail_pending().unwrap();
        }
        fn daily_active_day(&self) -> Option<Option<String>> {
            lock(&self.0).telemetry_daily_active_day().ok()
        }
        fn set_daily_active_day(&self, day: &str) -> bool {
            lock(&self.0).set_telemetry_daily_active_day(day).is_ok()
        }
        fn hourly_active_at(&self) -> Option<Option<i64>> {
            lock(&self.0).telemetry_hourly_active_at().ok()
        }
        fn set_hourly_active_at(&self, at_ms: i64) -> bool {
            lock(&self.0).set_telemetry_hourly_active_at(at_ms).is_ok()
        }
    }

    /// A fixed UTC-7 local zone so a local day can end before the UTC one.
    pub(crate) struct FakeClock(pub AtomicI64);
    impl FakeClock {
        pub(crate) fn at(ms: i64) -> Arc<Self> {
            Arc::new(Self(AtomicI64::new(ms)))
        }
        pub(crate) fn advance(&self, ms: i64) {
            self.0.fetch_add(ms, Ordering::SeqCst);
        }
    }
    impl Clock for FakeClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
        fn local_day(&self, ms: i64) -> String {
            let zone = jiff::tz::TimeZone::fixed(jiff::tz::offset(-7));
            jiff::Timestamp::from_millisecond(ms)
                .unwrap()
                .to_zoned(zone)
                .date()
                .to_string()
        }
    }

    type Hook = Arc<dyn Fn() + Send + Sync>;

    /// Records every request body; answers from a script (success when the
    /// script is empty). `during` runs inside each request, before it answers.
    #[derive(Default)]
    pub(crate) struct MockTransport {
        pub bodies: Mutex<Vec<Value>>,
        script: Mutex<VecDeque<Result<(), SendError>>>,
        during: Mutex<Option<Hook>>,
    }
    impl MockTransport {
        pub(crate) fn answering(script: &[Result<(), SendError>]) -> Arc<Self> {
            let mock = Self::default();
            *lock(&mock.script) = script.iter().copied().collect();
            Arc::new(mock)
        }
        pub(crate) fn during(&self, hook: impl Fn() + Send + Sync + 'static) {
            *lock(&self.during) = Some(Arc::new(hook));
        }
        pub(crate) fn requests(&self) -> usize {
            lock(&self.bodies).len()
        }
        pub(crate) fn events(&self) -> Vec<String> {
            lock(&self.bodies)
                .iter()
                .flat_map(|b| b["batch"].as_array().unwrap().clone())
                .map(|e| e["event"].as_str().unwrap().to_owned())
                .collect()
        }
    }
    impl Transport for MockTransport {
        fn send(&self, body: &[u8], cancel: &Cancel) -> Result<(), SendError> {
            lock(&self.bodies).push(serde_json::from_slice(body).unwrap());
            let hook = lock(&self.during).clone();
            if let Some(hook) = hook {
                hook();
            }
            let answer = lock(&self.script).pop_front().unwrap_or(Ok(()));
            // A cancelled request stops before an answer arrives, unless the
            // script says the answer was already accepted.
            if cancel.is_cancelled() && answer.is_err() {
                return Err(SendError::Cancelled);
            }
            answer
        }
    }

    pub(crate) const START: i64 = 1_800_000_000_000; // 2027-01-15T08:00:00Z, 01:00 at UTC-7

    pub(crate) fn config() -> PostHogConfig {
        PostHogConfig::new(Some("phc_test"), None).unwrap()
    }

    pub(crate) struct Rig {
        pub gate: Arc<Gate>,
        pub store: Arc<TestStore>,
        pub clock: Arc<FakeClock>,
        pub transport: Arc<MockTransport>,
        pub shared: Shared,
    }

    pub(crate) fn rig(consent: Consent, env_off: bool, transport: Arc<MockTransport>) -> Rig {
        rig_with(
            consent,
            env_off,
            transport,
            Store::open_in_memory().unwrap(),
        )
    }

    pub(crate) fn rig_with(
        consent: Consent,
        env_off: bool,
        transport: Arc<MockTransport>,
        store: Store,
    ) -> Rig {
        let gate = Arc::new(Gate::new(consent, env_off));
        let store = TestStore::new(store);
        let clock = FakeClock::at(START);
        let shared = Shared::new(
            gate.clone(),
            config(),
            transport.clone(),
            store.clone(),
            clock.clone(),
            OsVersion::parse("15.5"),
        );
        Rig {
            gate,
            store,
            clock,
            transport,
            shared,
        }
    }

    pub(crate) const ON: Consent = Consent {
        preference_on: true,
        disclosure_shown: true,
    };
    const OFF: Consent = Consent {
        preference_on: false,
        disclosure_shown: true,
    };

    fn drain(rig: &Rig) {
        while rig.shared.deliver_once() {}
    }

    #[test]
    fn no_request_and_no_backlog_while_the_gate_is_closed() {
        for (consent, env_off) in [(Consent::FRESH, false), (OFF, false), (ON, true)] {
            let rig = rig(consent, env_off, MockTransport::answering(&[]));
            rig.shared.on_opened();
            rig.shared.on_activity();
            drain(&rig);
            assert_eq!(rig.transport.requests(), 0);
            assert!(
                rig.store.ledger().is_empty(),
                "nothing is recorded or queued"
            );
            // Becoming eligible later sends nothing from before.
            rig.gate.set(ON);
            if !env_off {
                drain(&rig);
                assert_eq!(rig.transport.requests(), 0, "no replay of earlier attempts");
                rig.shared.on_activity();
                drain(&rig);
                assert_eq!(rig.transport.events(), ["daily_active", "hourly_active"]);
            }
        }
    }

    #[test]
    fn eligible_launch_sends_app_opened_and_daily_active_and_marks_them_sent() {
        let rig = rig(ON, false, MockTransport::answering(&[]));
        rig.shared.on_opened();
        drain(&rig);
        assert_eq!(rig.transport.events(), ["app_opened", "daily_active"]);
        assert_eq!(
            rig.store.ledger(),
            [
                ("app_opened".into(), DeliveryStatus::Sent),
                ("daily_active".into(), DeliveryStatus::Sent)
            ]
        );
    }

    #[test]
    fn daily_once_per_local_day_and_hourly_on_activity_with_a_one_hour_debounce() {
        let rig = rig(ON, false, MockTransport::answering(&[]));
        // 01:00 local: the first use of the day.
        rig.shared.on_activity();
        rig.clock.advance(10 * 60 * 1000);
        rig.shared.on_activity();
        drain(&rig);
        assert_eq!(rig.transport.events(), ["daily_active", "hourly_active"]);
        // Just under an hour after the first hourly: nothing.
        rig.clock.advance(50 * 60 * 1000 - 1);
        rig.shared.on_activity();
        drain(&rig);
        assert_eq!(rig.transport.events().len(), 2);
        // A full hour (02:00 local): one hourly.
        rig.clock.advance(1);
        rig.shared.on_activity();
        drain(&rig);
        assert_eq!(rig.transport.events().len(), 3);
        // Hours without any use send nothing.
        rig.clock.advance(15 * heartbeat::HOURLY_DEBOUNCE_MS);
        drain(&rig);
        assert_eq!(rig.transport.events().len(), 3);
        // 17:01 local is already the next day in UTC, not locally: hourly only.
        rig.clock.advance(60_000);
        rig.shared.on_activity();
        drain(&rig);
        assert_eq!(rig.transport.events()[3..], ["hourly_active".to_string()]);
        // Local midnight: a new local day.
        rig.clock
            .advance(6 * heartbeat::HOURLY_DEBOUNCE_MS + 59 * 60_000);
        rig.shared.on_activity();
        drain(&rig);
        assert_eq!(
            rig.transport.events()[4..],
            ["daily_active".to_string(), "hourly_active".to_string()]
        );
        // A restart on the same day does not send daily again.
        let reopened = Shared::new(
            rig.gate.clone(),
            config(),
            rig.transport.clone(),
            rig.store.clone(),
            rig.clock.clone(),
            OsVersion::parse("15.5"),
        );
        reopened.on_opened();
        while reopened.deliver_once() {}
        assert_eq!(rig.transport.events().last().unwrap(), "app_opened");
        assert_eq!(rig.transport.events().len(), 7);
    }

    #[test]
    fn a_heartbeat_dropped_while_off_is_not_counted_as_sent() {
        let rig = rig(OFF, false, MockTransport::answering(&[]));
        rig.shared.on_activity();
        assert_eq!(rig.store.daily_active_day(), Some(None));
        rig.gate.set(ON);
        rig.shared.on_activity();
        drain(&rig);
        assert_eq!(rig.transport.events(), ["daily_active", "hourly_active"]);
    }

    #[test]
    fn failures_retry_with_backoff_and_fail_after_the_last_attempt() {
        let failed = Err(SendError::Failed);
        let rig = rig(
            ON,
            false,
            MockTransport::answering(&[failed, failed, failed, failed]),
        );
        rig.shared.emit(TelemetryEvent::AppOpened);
        drain(&rig);
        assert_eq!(rig.transport.requests(), 1);
        assert_eq!(rig.store.ledger()[0].1, DeliveryStatus::Pending);
        for attempt in 1..MAX_ATTEMPTS {
            rig.clock.advance(retry_delay_ms(attempt) - 1);
            drain(&rig);
            assert_eq!(rig.transport.requests() as u32, attempt, "not due yet");
            rig.clock.advance(1);
            drain(&rig);
            assert_eq!(rig.transport.requests() as u32, attempt + 1);
        }
        assert_eq!(rig.store.ledger()[0].1, DeliveryStatus::Failed);
        rig.clock.advance(3_600_000);
        drain(&rig);
        assert_eq!(rig.transport.requests() as u32, MAX_ATTEMPTS);
    }

    #[test]
    fn a_retry_that_succeeds_is_sent_and_pending_and_failed_stay_distinct() {
        let rig = rig(
            ON,
            false,
            MockTransport::answering(&[Err(SendError::Failed), Ok(())]),
        );
        rig.shared.emit(TelemetryEvent::AppOpened);
        drain(&rig);
        rig.shared.emit(TelemetryEvent::HourlyActive);
        drain(&rig);
        // The new event was sent; the first waits for its retry.
        assert_eq!(
            rig.store.ledger(),
            [
                ("app_opened".into(), DeliveryStatus::Pending),
                ("hourly_active".into(), DeliveryStatus::Sent)
            ]
        );
        rig.clock.advance(FIRST_RETRY_MS);
        drain(&rig);
        assert_eq!(rig.store.ledger()[0].1, DeliveryStatus::Sent);
    }

    #[test]
    fn opting_out_drops_a_queued_retry_and_nothing_is_replayed_on_re_enable() {
        let rig = rig(
            ON,
            false,
            MockTransport::answering(&[Err(SendError::Failed)]),
        );
        rig.shared.emit(TelemetryEvent::AppOpened);
        drain(&rig);
        assert_eq!(rig.transport.requests(), 1);
        rig.gate.set(OFF);
        rig.shared.close();
        assert_eq!(rig.store.ledger()[0].1, DeliveryStatus::Failed);
        for _ in 0..5 {
            rig.clock.advance(3_600_000);
            drain(&rig);
        }
        assert_eq!(rig.transport.requests(), 1, "no request after opting out");
        rig.gate.set(ON);
        drain(&rig);
        assert_eq!(rig.transport.requests(), 1, "re-enabling replays nothing");
        rig.shared.emit(TelemetryEvent::HourlyActive);
        drain(&rig);
        assert_eq!(rig.transport.events(), ["app_opened", "hourly_active"]);
    }

    #[test]
    fn opting_out_during_a_request_cancels_it_and_its_late_answer_queues_nothing() {
        for late in [Err(SendError::Failed), Ok(())] {
            let rig = rig(ON, false, MockTransport::answering(&[late]));
            rig.shared.emit(TelemetryEvent::AppOpened);
            rig.shared.emit(TelemetryEvent::DailyActive);
            let batch = rig.shared.take_batch(rig.clock.now_ms()).unwrap();
            // The user turns telemetry off while the request is in flight.
            rig.gate.set(OFF);
            rig.shared.close();
            assert!(
                batch.cancel.is_cancelled(),
                "the in-flight request is cancelled"
            );
            // The transport's late callback arrives after the opt-out.
            let result = rig.transport.send(b"{\"batch\":[]}", &batch.cancel);
            rig.shared.finish(batch, result, rig.clock.now_ms());
            let statuses: Vec<_> = rig.store.ledger().into_iter().map(|r| r.1).collect();
            // A request the endpoint already accepted is recorded truthfully.
            let expected = if late.is_ok() {
                DeliveryStatus::Sent
            } else {
                DeliveryStatus::Failed
            };
            assert_eq!(statuses, [expected, expected]);
            rig.gate.set(ON);
            for _ in 0..5 {
                rig.clock.advance(3_600_000);
                drain(&rig);
            }
            assert_eq!(rig.transport.requests(), 1, "nothing is resent or replayed");
        }
    }

    #[test]
    fn the_gate_is_checked_again_immediately_before_the_request() {
        let rig = rig(ON, false, MockTransport::answering(&[]));
        rig.shared.emit(TelemetryEvent::AppOpened);
        let batch = rig.shared.take_batch(rig.clock.now_ms()).unwrap();
        // Closed between taking the batch and sending it, without the
        // sender being told: the request is still not made.
        rig.gate.set(OFF);
        let result = rig.shared.send(&batch);
        assert_eq!(result, Err(SendError::Cancelled));
        rig.shared.finish(batch, result, rig.clock.now_ms());
        assert_eq!(rig.transport.requests(), 0);
        assert_eq!(rig.store.ledger()[0].1, DeliveryStatus::Failed);
    }

    #[test]
    fn the_thread_stops_sending_when_opted_out_during_an_in_flight_request() {
        let transport = MockTransport::answering(&[]);
        let gate = Arc::new(Gate::new(ON, false));
        let store = TestStore::new(Store::open_in_memory().unwrap());
        let clock = FakeClock::at(START);
        let (entered, release) = (
            Arc::new(std::sync::Barrier::new(2)),
            Arc::new(std::sync::Barrier::new(2)),
        );
        {
            let (entered, release) = (entered.clone(), release.clone());
            transport.during(move || {
                entered.wait();
                release.wait();
            });
        }
        let sender = Sender::start(
            gate.clone(),
            config(),
            transport.clone(),
            store.clone(),
            clock.clone(),
        );
        sender.opened();
        entered.wait();
        // In flight: opt out, then let the request return.
        gate.set(OFF);
        sender.close();
        transport.during(|| {});
        release.wait();
        sender.activity();
        clock.advance(2 * 3_600_000);
        sender.activity();
        std::thread::sleep(Duration::from_millis(200));
        sender.shutdown();
        assert_eq!(transport.requests(), 1);
        assert!(
            store
                .ledger()
                .iter()
                .all(|(_, status)| *status != DeliveryStatus::Pending)
        );
    }

    #[test]
    fn a_full_queue_fails_new_events_instead_of_growing() {
        let rig = rig(ON, false, MockTransport::answering(&[]));
        for _ in 0..QUEUE_CAPACITY {
            assert!(rig.shared.emit(TelemetryEvent::HourlyActive));
        }
        assert!(!rig.shared.emit(TelemetryEvent::HourlyActive));
        let ledger = rig.store.ledger();
        assert_eq!(ledger.len(), QUEUE_CAPACITY + 1);
        assert_eq!(ledger.last().unwrap().1, DeliveryStatus::Failed);
        drain(&rig);
        assert_eq!(rig.transport.requests(), QUEUE_CAPACITY.div_ceil(BATCH_MAX));
    }

    #[test]
    fn the_ledger_never_holds_more_than_two_hundred_and_marks_sent_only_on_success() {
        let script: Vec<_> = (0..300)
            .map(|n| {
                if n % 3 == 0 {
                    Err(SendError::Failed)
                } else {
                    Ok(())
                }
            })
            .collect();
        let rig = rig(ON, false, MockTransport::answering(&script));
        for n in 0..250 {
            rig.shared.emit(TelemetryEvent::HourlyActive);
            drain(&rig);
            assert!(rig.store.ledger().len() <= 200, "after {n}");
        }
        let ledger = rig.store.ledger();
        assert_eq!(ledger.len(), 200);
        let sent = ledger
            .iter()
            .filter(|r| r.1 == DeliveryStatus::Sent)
            .count();
        let pending = ledger
            .iter()
            .filter(|r| r.1 == DeliveryStatus::Pending)
            .count();
        assert!(sent > 0 && pending > 0, "{sent} sent, {pending} pending");
        // Every accepted request's events, and only those, are marked sent.
        let accepted: usize = lock(&rig.transport.bodies)
            .iter()
            .zip(script.iter())
            .filter(|(_, answer)| answer.is_ok())
            .map(|(body, _)| body["batch"].as_array().unwrap().len())
            .sum();
        let all_sent = lock(&rig.store.0)
            .telemetry_ledger(1000)
            .unwrap()
            .iter()
            .filter(|r| r.status == DeliveryStatus::Sent)
            .count();
        assert!(all_sent <= accepted);
    }

    #[test]
    fn nothing_made_while_quitting_is_sent() {
        let rig = rig(ON, false, MockTransport::answering(&[]));
        rig.shared.signal(|inner| inner.stop = true);
        rig.shared.close();
        assert!(!rig.shared.emit(TelemetryEvent::HourlyActive));
        drain(&rig);
        assert_eq!(rig.transport.requests(), 0);
        assert!(rig.store.ledger().is_empty());
    }

    #[test]
    fn hourly_waits_a_full_hour_across_a_restart() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("xtrace.db");
        let transport = MockTransport::answering(&[]);
        let first = rig_with(ON, false, transport.clone(), Store::open(&path).unwrap());
        first.shared.on_activity();
        drain(&first);
        assert_eq!(transport.events(), ["daily_active", "hourly_active"]);
        drop(first);
        // Restarted half an hour later on a reopened store: no second hourly.
        let second = rig_with(ON, false, transport.clone(), Store::open(&path).unwrap());
        second.clock.advance(30 * 60_000);
        second.shared.on_activity();
        drain(&second);
        assert_eq!(transport.events().len(), 2, "{:?}", transport.events());
        // A full hour after the first one: sent.
        second.clock.advance(30 * 60_000);
        second.shared.on_activity();
        drain(&second);
        assert_eq!(transport.events()[2..], ["hourly_active".to_string()]);
    }

    #[test]
    fn retry_delays_grow_and_stay_bounded() {
        assert_eq!(retry_delay_ms(1), 30_000);
        assert_eq!(retry_delay_ms(2), 120_000);
        assert_eq!(retry_delay_ms(3), 480_000);
        assert!(retry_delay_ms(u32::MAX) > 0);
    }
}
