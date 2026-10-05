//! The manual pull-request refresh backend: selection, bounds, cancellation,
//! shutdown, persistence and the refresh event.
//!
//! Every attempt here is synthetic. Where a process runs at all, it is a
//! `#!/bin/sh` fixture this test wrote into its own temporary directory. No
//! real GitHub CLI is resolved or launched, no network request is made, no
//! credential or configuration file is read, and no real pull request is
//! contacted. Every database is a temporary file this test created.
#![cfg(unix)]

use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tempfile::TempDir;
use xt_probes::gh::{AttemptClock, GhClientError};
use xt_store::{
    SessionMeta, SessionSource, Store,
    pr_link::{
        PrConfidence, PrIdentity, PrLinkObservation, PrRefreshError, PrState, RefreshFailure,
        RefreshOutcome, RefreshSuccess,
    },
};
use xtrace_desktop::{
    dto::{
        PrAttemptOutcome, PrPersistence, PrPersistenceError, PrRefreshErrorCode, PrRefreshReport,
        PrRefreshStatusReport, PrRefreshWriteReport, PrSkipReason, PrStateReport,
    },
    gh_cli::{self, GhUnavailable},
    pr_refresh::{
        ATTEMPT_BUDGET, Attempt, AttemptRequest, Budget, MAX_SELECTION, PrRefreshService, Publish,
        RefreshError, RefreshStorage, TOTAL_BUDGET,
    },
    state::{AppState, StartupOptions, StateError},
};

const ALPHA: &str = "https://github.com/octo-org/alpha/pull/1";
const BETA: &str = "https://github.com/octo-org/beta/pull/2";
const GAMMA: &str = "https://github.com/octo-org/gamma/pull/3";
/// Attempt times stay well inside the recordable range and stay ordered.
const BASE_ATTEMPT: i64 = 1_700_000_000_000;

fn identity(url: &str) -> PrIdentity {
    PrIdentity::from_url(url).unwrap()
}

// ---------------------------------------------------------------- harness --

/// A live application state over a temporary database seeded with linked
/// pull requests, plus the database path so a test can read it independently.
struct App {
    state: Arc<AppState>,
    database: PathBuf,
    _root: TempDir,
}

fn app(urls: &[&str]) -> App {
    let root = TempDir::new().unwrap();
    // macOS resolves a temporary root through /private/var; the destination
    // check compares real paths, so resolve once here.
    let base = root.path().canonicalize().unwrap();
    let data = base.join("data");
    let home = base.join("home");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let database = data.join("xtrace.db");
    {
        let mut store = Store::open(&database).unwrap();
        for session in ["session-a", "session-b"] {
            store
                .upsert_session(
                    &SessionMeta::new(session, "claude", SessionSource::Fixture),
                    false,
                )
                .unwrap();
        }
        for url in urls {
            store
                .record_pr_link(&PrLinkObservation {
                    session_id: "session-a".into(),
                    pull_request: identity(url),
                    confidence: PrConfidence::Exact,
                    first_seen_at: 10,
                    last_seen_at: 20,
                })
                .unwrap();
        }
    }
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(data),
            ..Default::default()
        },
        || panic!("the default data directory must not be resolved"),
        || Ok(home),
    )
    .unwrap();
    App {
        state: Arc::new(state),
        database,
        _root: root,
    }
}

impl App {
    /// The stored IDs of the listed pull requests, in list order.
    fn ids(&self) -> Vec<i64> {
        self.state
            .pr_list()
            .unwrap()
            .rows
            .iter()
            .map(|row| row.pull_request.id)
            .collect()
    }

    fn id(&self, url: &str) -> i64 {
        let wanted = identity(url).url();
        self.state
            .pr_list()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.pull_request.url == wanted)
            .expect("the pull request is listed")
            .pull_request
            .id
    }

    /// Read the stored rows through an independent connection, so an
    /// assertion never depends on the state the batch was using.
    fn stored(&self) -> HashMap<String, xt_store::pr_link::StoredPullRequest> {
        Store::open(&self.database)
            .unwrap()
            .all_pull_requests()
            .unwrap()
            .into_iter()
            .map(|row| (row.identity.url(), row))
            .collect()
    }
}

/// One attempt this test observed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Call {
    url: String,
    deadline: Duration,
}

type Log = Arc<Mutex<Vec<Call>>>;

fn log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

fn calls(log: &Log) -> Vec<Call> {
    log.lock().unwrap().clone()
}

/// An injected attempt that records every call and answers with `respond`,
/// which also receives the zero-based index of the call.
fn attempts<F>(log: &Log, respond: F) -> Attempt
where
    F: Fn(&AttemptRequest<'_>, usize) -> Result<RefreshOutcome, GhClientError>
        + Send
        + Sync
        + 'static,
{
    let log = Arc::clone(log);
    Arc::new(move |request: &AttemptRequest<'_>| {
        let index = {
            let mut log = log.lock().unwrap();
            log.push(Call {
                url: request.identity.url(),
                deadline: request.deadline,
            });
            log.len() - 1
        };
        respond(request, index)
    })
}

fn success(request: &AttemptRequest<'_>) -> Result<RefreshOutcome, GhClientError> {
    Ok(RefreshOutcome::Success(RefreshSuccess {
        pull_request: request.identity.clone(),
        attempted_at: request.clock.attempted_at(),
        title: format!("Synthetic title {}", request.identity.number()),
        state: PrState::Open,
        merged_at: None,
        additions: 7,
        deletions: 3,
        head_ref_name: "feature/synthetic".into(),
    }))
}

fn failure(
    request: &AttemptRequest<'_>,
    error: PrRefreshError,
) -> Result<RefreshOutcome, GhClientError> {
    Ok(RefreshOutcome::Failure(RefreshFailure {
        pull_request: request.identity.clone(),
        attempted_at: request.clock.attempted_at(),
        error,
    }))
}

/// A clock whose reading advances by a millisecond per attempt, so the
/// storage layer's attempt ordering is exercised deterministically.
struct StepClock(AtomicI64);

impl StepClock {
    fn from(start: i64) -> Arc<Self> {
        Arc::new(Self(AtomicI64::new(start)))
    }
}

impl AttemptClock for StepClock {
    fn attempted_at(&self) -> i64 {
        self.0.fetch_add(1, Ordering::SeqCst)
    }
}

/// A clock pinned to one reading.
struct PinnedClock(i64);

impl AttemptClock for PinnedClock {
    fn attempted_at(&self) -> i64 {
        self.0
    }
}

/// Counts refresh events.
fn publisher() -> (Publish, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&count);
    (
        Arc::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
        }),
        count,
    )
}

fn silent() -> Publish {
    Arc::new(|| {})
}

/// A gate another thread opens, used instead of a channel because an attempt
/// must be callable from any thread.
#[derive(Clone, Default)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    fn wait(&self) {
        let (lock, signal) = &*self.0;
        let mut open = lock.lock().unwrap();
        while !*open {
            open = signal.wait(open).unwrap();
        }
    }
    fn open(&self) {
        let (lock, signal) = &*self.0;
        *lock.lock().unwrap() = true;
        signal.notify_all();
    }
    /// Wait for the gate within a bound; false if it never opened.
    fn wait_within(&self, limit: Duration) -> bool {
        let (lock, signal) = &*self.0;
        let (open, _) = signal
            .wait_timeout_while(lock.lock().unwrap(), limit, |open| !*open)
            .unwrap();
        *open
    }
}

fn await_active(service: &PrRefreshService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !service.is_active() {
        assert!(Instant::now() < deadline, "the batch never became active");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn outcomes(report: &PrRefreshReport) -> Vec<PrAttemptOutcome> {
    report.rows.iter().map(|row| row.outcome).collect()
}

fn skipped(reason: PrSkipReason) -> PrAttemptOutcome {
    PrAttemptOutcome::Skipped { reason }
}

fn applied() -> PrAttemptOutcome {
    succeeded(PrRefreshWriteReport::Applied)
}

fn succeeded(write: PrRefreshWriteReport) -> PrAttemptOutcome {
    PrAttemptOutcome::Succeeded {
        persistence: PrPersistence::Recorded { write },
    }
}

fn failed(error: PrRefreshErrorCode, write: PrRefreshWriteReport) -> PrAttemptOutcome {
    PrAttemptOutcome::Failed {
        error,
        persistence: PrPersistence::Recorded { write },
    }
}

/// An attempt that ran and answered, whose result storage would not keep.
fn unrecorded(outcome: PrAttemptOutcome, reason: PrPersistenceError) -> PrAttemptOutcome {
    match outcome {
        PrAttemptOutcome::Succeeded { .. } => PrAttemptOutcome::Succeeded {
            persistence: PrPersistence::NotRecorded { reason },
        },
        PrAttemptOutcome::Failed { error, .. } => PrAttemptOutcome::Failed {
            error,
            persistence: PrPersistence::NotRecorded { reason },
        },
        skipped => skipped,
    }
}

// -------------------------------------------------------------- selection --

#[test]
fn only_stored_identifiers_are_accepted_and_a_refused_selection_runs_nothing() {
    let app = app(&[ALPHA, BETA]);
    let log = log();
    let (publish, events) = publisher();
    let service =
        PrRefreshService::injected(attempts(&log, |request, _| success(request)), publish);
    let ids = app.ids();
    let refused: Vec<(Vec<i64>, &str)> = vec![
        (Vec::new(), "EmptySelection"),
        (
            (1..=(MAX_SELECTION as i64 + 1)).collect(),
            "SelectionTooLarge",
        ),
        (vec![0], "InvalidSelection"),
        (vec![-1], "InvalidSelection"),
        (vec![i64::MAX], "InvalidSelection"),
        (vec![1_i64 << 53], "InvalidSelection"),
        (vec![ids[0], ids[0]], "DuplicateSelection"),
        (vec![ids[0], ids[1], ids[0]], "DuplicateSelection"),
    ];
    for (selection, expected) in refused {
        let error = service
            .refresh(&app.state, &selection)
            .expect_err("the selection must be refused");
        let named = match error {
            RefreshError::EmptySelection => "EmptySelection",
            RefreshError::SelectionTooLarge => "SelectionTooLarge",
            RefreshError::InvalidSelection => "InvalidSelection",
            RefreshError::DuplicateSelection => "DuplicateSelection",
            other => panic!("unexpected refusal: {other}"),
        };
        assert_eq!(named, expected, "selection {selection:?}");
    }
    // A refused selection never reaches an attempt, a write or the event.
    assert!(calls(&log).is_empty());
    assert_eq!(events.load(Ordering::SeqCst), 0);
    for row in app.state.pr_list().unwrap().rows {
        assert_eq!(row.last_attempted_at_ms, None);
        assert_eq!(row.refreshed_at_ms, None);
    }
    // Exactly the documented bound is accepted.
    assert_eq!(MAX_SELECTION, 20);
    assert_eq!(TOTAL_BUDGET, Duration::from_secs(120));
    assert_eq!(ATTEMPT_BUDGET, Duration::from_secs(30));
    assert_eq!(
        Budget::default(),
        Budget {
            total: TOTAL_BUDGET,
            attempt: ATTEMPT_BUDGET,
            minimum: Duration::from_secs(1),
            shutdown: Duration::from_secs(6),
        }
    );
}

#[test]
fn an_identifier_that_names_no_stored_pull_request_is_skipped() {
    let app = app(&[ALPHA]);
    let log = log();
    let (publish, events) = publisher();
    let service =
        PrRefreshService::injected(attempts(&log, |request, _| success(request)), publish);
    let unknown = app.ids().iter().max().copied().unwrap() + 1_000;
    let report = service.refresh(&app.state, &[unknown]).unwrap();
    assert_eq!(outcomes(&report), vec![skipped(PrSkipReason::NotStored)]);
    assert_eq!(report.rows[0].id, unknown);
    // An unknown ID has no identity to report, and nothing ran for it.
    assert!(report.rows[0].pull_request.is_none());
    assert_eq!(
        (report.requested, report.attempted, report.skipped),
        (1, 0, 1)
    );
    assert!(!report.committed);
    assert!(calls(&log).is_empty());
    assert_eq!(events.load(Ordering::SeqCst), 0);
}

#[test]
fn a_stored_pull_request_that_no_session_links_is_skipped_not_attempted() {
    let app = app(&[ALPHA, BETA]);
    let alpha = app.id(ALPHA);
    let beta = app.id(BETA);
    // Unlink beta through an independent connection: no public writer removes
    // a link, and the guarantee must hold if one ever does.
    unlink(&app.database, BETA);
    let log = log();
    let service =
        PrRefreshService::injected(attempts(&log, |request, _| success(request)), silent());
    let report = service.refresh(&app.state, &[beta, alpha]).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![skipped(PrSkipReason::NotLinked), applied()]
    );
    // The unlinked identity is still named, so a caller can explain the skip.
    assert_eq!(
        report.rows[0].pull_request.as_ref().unwrap().url,
        identity(BETA).url()
    );
    assert_eq!(
        calls(&log)
            .into_iter()
            .map(|call| call.url)
            .collect::<Vec<_>>(),
        vec![identity(ALPHA).url()]
    );
    // It also leaves the list, which only reports pull requests still linked.
    let listed: Vec<String> = app
        .state
        .pr_list()
        .unwrap()
        .rows
        .into_iter()
        .map(|row| row.pull_request.url)
        .collect();
    assert_eq!(listed, vec![identity(ALPHA).url()]);
    assert_eq!(app.stored()[&identity(BETA).url()].refreshed_at, None);
}

/// Remove every link of one pull request, which the public writers cannot do.
fn unlink(database: &Path, url: &str) {
    let store = Store::open(database).unwrap();
    let id = store
        .all_pull_requests()
        .unwrap()
        .into_iter()
        .find(|row| row.identity.url() == identity(url).url())
        .unwrap()
        .id;
    drop(store);
    let connection = rusqlite::Connection::open(database).unwrap();
    connection
        .execute("DELETE FROM pr_links WHERE pr_id=?1", [id])
        .unwrap();
}

// ------------------------------------------------------------ the batch ----

#[test]
fn a_batch_attempts_in_selection_order_and_persists_every_result() {
    let app = app(&[ALPHA, BETA, GAMMA]);
    let log = log();
    let (publish, events) = publisher();
    let service =
        PrRefreshService::injected(attempts(&log, |request, _| success(request)), publish)
            .with_clock(StepClock::from(BASE_ATTEMPT));
    let selection = vec![app.id(GAMMA), app.id(ALPHA), app.id(BETA)];
    let report = service.refresh(&app.state, &selection).unwrap();
    assert_eq!(outcomes(&report), vec![applied(), applied(), applied()]);
    assert_eq!(
        (
            report.requested,
            report.attempted,
            report.succeeded,
            report.failed,
            report.skipped
        ),
        (3, 3, 3, 0, 0)
    );
    assert!(report.committed && !report.cancelled);
    // The selection's order is the report's order and the attempt order, and
    // each attempt is argued with the canonical identity, never a spelling a
    // caller supplied: the caller only ever supplied an integer.
    assert_eq!(
        report
            .rows
            .iter()
            .map(|row| row.pull_request.as_ref().unwrap().url.clone())
            .collect::<Vec<_>>(),
        vec![
            identity(GAMMA).url(),
            identity(ALPHA).url(),
            identity(BETA).url()
        ]
    );
    assert_eq!(
        calls(&log),
        vec![
            Call {
                url: identity(GAMMA).url(),
                deadline: ATTEMPT_BUDGET
            },
            Call {
                url: identity(ALPHA).url(),
                deadline: ATTEMPT_BUDGET
            },
            Call {
                url: identity(BETA).url(),
                deadline: ATTEMPT_BUDGET
            },
        ]
    );
    assert_eq!(events.load(Ordering::SeqCst), 1);
    // Every refresh-owned field is what the attempt reported.
    let stored = app.stored();
    for url in [ALPHA, BETA, GAMMA] {
        let row = &stored[&identity(url).url()];
        assert_eq!(
            row.title.as_deref(),
            Some(format!("Synthetic title {}", identity(url).number()).as_str())
        );
        assert_eq!(row.state, Some(PrState::Open));
        assert_eq!(row.merged_at, None);
        assert_eq!((row.additions, row.deletions), (Some(7), Some(3)));
        assert_eq!(row.head_ref_name.as_deref(), Some("feature/synthetic"));
        assert_eq!(row.refresh_error, None);
        assert_eq!(row.refreshed_at, row.last_attempted_at);
    }
    let listed = app.state.pr_list().unwrap();
    assert!(
        listed
            .rows
            .iter()
            .all(|row| matches!(row.status, PrRefreshStatusReport::Refreshed))
    );
    assert!(listed.rows.iter().all(|row| row.linked_sessions == 1));
    assert_eq!(listed.rows[0].state, Some(PrStateReport::Open));
}

#[test]
fn a_failure_is_recorded_without_losing_the_last_successful_metadata() {
    let app = app(&[ALPHA]);
    let alpha = app.id(ALPHA);
    let clock = StepClock::from(BASE_ATTEMPT);
    let (publish, events) = publisher();
    let first = PrRefreshService::injected(
        attempts(&log(), |request, _| success(request)),
        Arc::clone(&publish),
    )
    .with_clock(Arc::clone(&clock) as Arc<dyn AttemptClock + Send + Sync>);
    first.refresh(&app.state, &[alpha]).unwrap();
    let refreshed_at = app.stored()[&identity(ALPHA).url()].refreshed_at;
    assert!(refreshed_at.is_some());

    let second = PrRefreshService::injected(
        attempts(&log(), |request, _| {
            failure(request, PrRefreshError::ExecutionFailed)
        }),
        publish,
    )
    .with_clock(clock as Arc<dyn AttemptClock + Send + Sync>);
    let report = second.refresh(&app.state, &[alpha]).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![failed(
            PrRefreshErrorCode::ExecutionFailed,
            PrRefreshWriteReport::Applied
        )]
    );
    assert_eq!(
        (report.attempted, report.failed, report.succeeded),
        (1, 1, 0)
    );
    let row = app.stored()[&identity(ALPHA).url()].clone();
    // The earlier success survives the failure; only the attempt columns move.
    assert_eq!(row.title.as_deref(), Some("Synthetic title 1"));
    assert_eq!(row.state, Some(PrState::Open));
    assert_eq!(row.refreshed_at, refreshed_at);
    assert!(row.last_attempted_at > refreshed_at);
    assert_eq!(row.refresh_error, Some(PrRefreshError::ExecutionFailed));
    let listed = app.state.pr_list().unwrap();
    assert_eq!(
        listed.rows[0].status,
        PrRefreshStatusReport::FailedAfterRefresh {
            error: PrRefreshErrorCode::ExecutionFailed
        }
    );
    assert_eq!(listed.rows[0].title.as_deref(), Some("Synthetic title 1"));
    assert_eq!(events.load(Ordering::SeqCst), 2);
}

#[test]
fn a_failure_before_any_success_reports_no_metadata_at_all() {
    let app = app(&[ALPHA]);
    let (publish, events) = publisher();
    let service = PrRefreshService::injected(
        attempts(&log(), |request, _| {
            failure(request, PrRefreshError::Unavailable)
        }),
        publish,
    )
    .with_clock(StepClock::from(BASE_ATTEMPT));
    service.refresh(&app.state, &[app.id(ALPHA)]).unwrap();
    let listed = app.state.pr_list().unwrap();
    assert_eq!(
        listed.rows[0].status,
        PrRefreshStatusReport::FailedNeverRefreshed {
            error: PrRefreshErrorCode::Unavailable
        }
    );
    assert_eq!(listed.rows[0].title, None);
    assert_eq!(listed.rows[0].refreshed_at_ms, None);
    assert!(listed.rows[0].last_attempted_at_ms.is_some());
    assert_eq!(events.load(Ordering::SeqCst), 1);
}

#[test]
fn a_stale_or_identical_result_commits_nothing_and_emits_nothing() {
    let app = app(&[ALPHA]);
    let alpha = app.id(ALPHA);
    let (publish, events) = publisher();
    let current = PrRefreshService::injected(
        attempts(&log(), |request, _| success(request)),
        Arc::clone(&publish),
    )
    .with_clock(Arc::new(PinnedClock(BASE_ATTEMPT)));
    assert!(current.refresh(&app.state, &[alpha]).unwrap().committed);
    assert_eq!(events.load(Ordering::SeqCst), 1);

    // The very same result at the very same attempt time changes nothing.
    let replay = current.refresh(&app.state, &[alpha]).unwrap();
    assert_eq!(
        outcomes(&replay),
        vec![succeeded(PrRefreshWriteReport::Unchanged)]
    );
    assert!(!replay.committed);

    // An older attempt is stale: storage keeps the newer stored result.
    let older = PrRefreshService::injected(
        attempts(&log(), |request, _| success(request)),
        Arc::clone(&publish),
    )
    .with_clock(Arc::new(PinnedClock(BASE_ATTEMPT - 5_000)));
    let stale = older.refresh(&app.state, &[alpha]).unwrap();
    assert_eq!(
        outcomes(&stale),
        vec![succeeded(PrRefreshWriteReport::Stale)]
    );
    assert!(!stale.committed);
    assert_eq!(
        app.stored()[&identity(ALPHA).url()].refreshed_at,
        Some(BASE_ATTEMPT)
    );
    // Only the one committing batch emitted.
    assert_eq!(events.load(Ordering::SeqCst), 1);
}

#[test]
fn the_event_is_emitted_once_and_only_after_the_change_is_committed() {
    let app = app(&[ALPHA, BETA]);
    let database = app.database.clone();
    let observed: Arc<Mutex<Vec<Vec<Option<String>>>>> = Arc::new(Mutex::new(Vec::new()));
    let publish: Publish = {
        let observed = Arc::clone(&observed);
        Arc::new(move || {
            // Read the committed rows from an independent connection at the
            // moment the event is published.
            let titles = Store::open(&database)
                .unwrap()
                .all_pull_requests()
                .unwrap()
                .into_iter()
                .map(|row| row.title)
                .collect();
            observed.lock().unwrap().push(titles);
        })
    };
    let service =
        PrRefreshService::injected(attempts(&log(), |request, _| success(request)), publish)
            .with_clock(StepClock::from(BASE_ATTEMPT));
    service
        .refresh(&app.state, &[app.id(ALPHA), app.id(BETA)])
        .unwrap();
    let observed = observed.lock().unwrap().clone();
    assert_eq!(observed.len(), 1, "one batch emits one event");
    assert_eq!(
        observed[0],
        vec![
            Some("Synthetic title 1".to_owned()),
            Some("Synthetic title 2".to_owned())
        ],
        "every committed row is readable before the event is published"
    );
}

// --------------------------------------------------------- one at a time ---

#[test]
fn an_overlapping_batch_is_refused_and_the_active_slot_is_always_released() {
    let app = app(&[ALPHA, BETA]);
    let gate = Gate::default();
    let held = gate.clone();
    let service = Arc::new(
        PrRefreshService::injected(
            attempts(&log(), move |request, index| {
                if index == 0 {
                    held.wait();
                }
                success(request)
            }),
            silent(),
        )
        .with_clock(StepClock::from(BASE_ATTEMPT)),
    );
    let running = {
        let service = Arc::clone(&service);
        let state = Arc::clone(&app.state);
        let selection = vec![app.id(ALPHA), app.id(BETA)];
        std::thread::spawn(move || service.refresh(&state, &selection).unwrap())
    };
    await_active(&service);
    // A second batch is refused rather than queued, and refusing it neither
    // cancels nor disturbs the one that is running.
    let refused = service.refresh(&app.state, &[app.id(ALPHA)]);
    assert!(matches!(refused, Err(RefreshError::AlreadyRunning)));
    gate.open();
    let report = running.join().unwrap();
    assert_eq!(outcomes(&report), vec![applied(), applied()]);
    assert!(!report.cancelled);
    assert!(!service.is_active());
    // The slot is free again for an ordinary batch.
    assert!(service.refresh(&app.state, &[app.id(ALPHA)]).is_ok());
}

#[test]
fn a_panic_inside_a_batch_still_releases_the_active_slot() {
    let app = app(&[ALPHA]);
    let service = PrRefreshService::injected(
        attempts(&log(), |_, _| panic!("this attempt panics deliberately")),
        silent(),
    )
    .with_clock(StepClock::from(BASE_ATTEMPT));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = service.refresh(&app.state, &[app.id(ALPHA)]);
    }));
    std::panic::set_hook(previous);
    assert!(panicked.is_err());
    assert!(!service.is_active(), "the guard released the slot");
    // And the service is usable again.
    let service =
        PrRefreshService::injected(attempts(&log(), |request, _| success(request)), silent())
            .with_clock(StepClock::from(BASE_ATTEMPT));
    assert!(service.refresh(&app.state, &[app.id(ALPHA)]).is_ok());
}

#[test]
fn other_state_reads_keep_working_while_a_batch_is_blocked_on_an_attempt() {
    let app = app(&[ALPHA, BETA]);
    let gate = Gate::default();
    let held = gate.clone();
    let service = Arc::new(
        PrRefreshService::injected(
            attempts(&log(), move |request, index| {
                if index == 0 {
                    held.wait();
                }
                success(request)
            }),
            silent(),
        )
        .with_clock(StepClock::from(BASE_ATTEMPT)),
    );
    let running = {
        let service = Arc::clone(&service);
        let state = Arc::clone(&app.state);
        let selection = vec![app.id(ALPHA), app.id(BETA)];
        std::thread::spawn(move || service.refresh(&state, &selection).unwrap())
    };
    await_active(&service);
    // The state lock was released before the attempt started, so every other
    // command answers while the batch is blocked.
    let (sender, receiver) = mpsc::channel();
    {
        let state = Arc::clone(&app.state);
        std::thread::spawn(move || {
            let counts = state.db_counts().is_ok();
            let listed = state.pr_list().map(|list| list.rows.len());
            let info = state.app_info().name;
            let _ = sender.send((counts, listed, info));
        });
    }
    let (counts, listed, info) = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("state reads must not wait for the batch");
    assert!(counts);
    assert_eq!(listed.unwrap(), 2);
    assert_eq!(info, "XTrace Desktop");
    gate.open();
    assert_eq!(
        outcomes(&running.join().unwrap()),
        vec![applied(), applied()]
    );
}

// -------------------------------------------------------- cancellation -----

#[test]
fn a_cancellation_before_the_first_attempt_leaves_everything_unattempted() {
    let app = app(&[ALPHA, BETA, GAMMA]);
    let log = log();
    let (publish, events) = publisher();
    let service =
        PrRefreshService::injected(attempts(&log, |request, _| success(request)), publish)
            .with_clock(StepClock::from(BASE_ATTEMPT))
            // A cancellation that arrives after the selection is resolved and
            // before the first attempt starts.
            .with_before_attempts(Arc::new(|cancel| cancel.cancel()));
    let selection = app.ids();
    let report = service.refresh(&app.state, &selection).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            skipped(PrSkipReason::Cancelled),
            skipped(PrSkipReason::Cancelled),
            skipped(PrSkipReason::Cancelled)
        ]
    );
    assert!(report.cancelled && !report.committed);
    assert_eq!((report.attempted, report.skipped), (0, 3));
    assert!(calls(&log).is_empty(), "nothing ran");
    assert_eq!(events.load(Ordering::SeqCst), 0);
    for row in app.state.pr_list().unwrap().rows {
        assert_eq!(row.last_attempted_at_ms, None);
    }
}

#[test]
fn a_cancellation_between_attempts_keeps_what_was_done_and_skips_the_rest() {
    let app = app(&[ALPHA, BETA, GAMMA]);
    let log = log();
    let (publish, events) = publisher();
    let service = PrRefreshService::injected(
        attempts(&log, |request, index| {
            if index == 0 {
                // The attempt cancels its own batch, exactly as a cancel
                // arriving from the frontend between two attempts would.
                request.cancel.cancel();
            }
            success(request)
        }),
        publish,
    )
    .with_clock(StepClock::from(BASE_ATTEMPT));
    let selection = app.ids();
    let report = service.refresh(&app.state, &selection).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            applied(),
            skipped(PrSkipReason::Cancelled),
            skipped(PrSkipReason::Cancelled)
        ]
    );
    assert!(report.cancelled && report.committed);
    assert_eq!(
        (report.attempted, report.succeeded, report.skipped),
        (1, 1, 2)
    );
    assert_eq!(calls(&log).len(), 1);
    // The committed first result still emits; the skipped two are untouched.
    assert_eq!(events.load(Ordering::SeqCst), 1);
    let rows = app.state.pr_list().unwrap().rows;
    assert!(rows[0].last_attempted_at_ms.is_some());
    assert_eq!(rows[1].last_attempted_at_ms, None);
    assert_eq!(rows[2].last_attempted_at_ms, None);
}

// ------------------------------------------------------------- shutdown ----

#[test]
fn shutdown_cancels_the_running_batch_and_refuses_every_later_one() {
    let app = app(&[ALPHA, BETA, GAMMA]);
    let log = log();
    // Opened from inside the first attempt. The batch is active before its
    // first attempt starts, and a shutdown in that gap correctly skips the
    // whole selection; this test is about the attempt that was running.
    let entered = Gate::default();
    let service = Arc::new(
        PrRefreshService::injected(
            attempts(&log, {
                let entered = entered.clone();
                move |request, index| {
                    if index == 0 {
                        entered.open();
                        // Stop as soon as the batch is cancelled, as the bounded
                        // client does when its child is killed.
                        while !request.cancel.is_cancelled() {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        return failure(request, PrRefreshError::Cancelled);
                    }
                    success(request)
                }
            }),
            silent(),
        )
        .with_clock(StepClock::from(BASE_ATTEMPT)),
    );
    let running = {
        let service = Arc::clone(&service);
        let state = Arc::clone(&app.state);
        let selection = app.ids();
        std::thread::spawn(move || service.refresh(&state, &selection).unwrap())
    };
    assert!(
        entered.wait_within(Duration::from_secs(10)),
        "the first attempt never started"
    );
    assert!(
        service.shutdown(),
        "shutdown waits for the cancelled batch to stop"
    );
    let report = running.join().unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            failed(PrRefreshErrorCode::Cancelled, PrRefreshWriteReport::Applied),
            skipped(PrSkipReason::Cancelled),
            skipped(PrSkipReason::Cancelled),
        ]
    );
    assert!(!service.is_active());
    // Storage is still open here: the cancelled attempt's result was written
    // before the application closes the database.
    assert_eq!(
        app.stored()[&identity(ALPHA).url()].refresh_error,
        Some(PrRefreshError::Cancelled)
    );
    assert!(matches!(
        service.refresh(&app.state, &app.ids()),
        Err(RefreshError::Stopped)
    ));
    // Shutting down twice is harmless, as the application's exit path may.
    assert!(service.shutdown());
}

#[test]
fn nothing_is_written_once_storage_has_closed() {
    let app = app(&[ALPHA, BETA, GAMMA]);
    let log = log();
    let (publish, events) = publisher();
    let closing = Arc::clone(&app.state);
    let service = PrRefreshService::injected(
        attempts(&log, move |request, index| {
            if index == 0 {
                // The application closed the database between the attempt and
                // its write, which is what shutdown does when a batch outlives
                // the bounded wait.
                closing.shutdown();
            }
            success(request)
        }),
        publish,
    )
    .with_clock(StepClock::from(BASE_ATTEMPT));
    let selection = app.ids();
    let report = service.refresh(&app.state, &selection).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            // The first attempt executed; only its result is missing.
            unrecorded(applied(), PrPersistenceError::StorageClosed),
            skipped(PrSkipReason::StorageUnavailable),
            skipped(PrSkipReason::StorageUnavailable),
        ]
    );
    assert_eq!(
        (report.attempted, report.unrecorded, report.skipped),
        (1, 1, 2)
    );
    assert!(!report.committed);
    assert_eq!(events.load(Ordering::SeqCst), 0);
    // Only the first pull request was attempted at all; the rest were not.
    assert_eq!(calls(&log).len(), 1);
    // The database on disk carries no attempt.
    for row in app.stored().values() {
        assert_eq!(row.last_attempted_at, None);
        assert_eq!(row.refreshed_at, None);
        assert_eq!(row.refresh_error, None);
    }
    assert!(matches!(app.state.pr_list(), Err(StateError::Closed)));
}

// --------------------------------------------------------------- budget ----

#[test]
fn a_spent_budget_skips_the_rest_instead_of_failing_them() {
    let app = app(&[ALPHA, BETA, GAMMA]);
    let log = log();
    let service = PrRefreshService::injected(
        attempts(&log, |request, _| {
            std::thread::sleep(Duration::from_millis(200));
            success(request)
        }),
        silent(),
    )
    .with_clock(StepClock::from(BASE_ATTEMPT))
    .with_budget(Budget {
        total: Duration::from_millis(250),
        attempt: Duration::from_millis(200),
        minimum: Duration::from_millis(100),
        shutdown: Duration::from_secs(1),
    });
    let selection = app.ids();
    let report = service.refresh(&app.state, &selection).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            applied(),
            skipped(PrSkipReason::BudgetExhausted),
            skipped(PrSkipReason::BudgetExhausted)
        ]
    );
    assert!(!report.cancelled, "a spent budget is not a cancellation");
    assert_eq!((report.attempted, report.skipped), (1, 2));
    // The one attempt never gets more than the per-attempt bound.
    assert_eq!(calls(&log).len(), 1);
    assert_eq!(calls(&log)[0].deadline, Duration::from_millis(200));
    // The unattempted two are unchanged, not failed.
    let rows = app.state.pr_list().unwrap().rows;
    assert_eq!(rows[1].last_attempted_at_ms, None);
    assert_eq!(rows[2].last_attempted_at_ms, None);
}

#[test]
fn an_attempt_never_gets_more_than_the_batch_has_left() {
    let app = app(&[ALPHA, BETA]);
    let log = log();
    let service = PrRefreshService::injected(
        attempts(&log, |request, _| {
            std::thread::sleep(Duration::from_millis(120));
            success(request)
        }),
        silent(),
    )
    .with_clock(StepClock::from(BASE_ATTEMPT))
    .with_budget(Budget {
        total: Duration::from_millis(400),
        attempt: Duration::from_secs(30),
        minimum: Duration::from_millis(10),
        shutdown: Duration::from_secs(1),
    });
    let selection = app.ids();
    service.refresh(&app.state, &selection).unwrap();
    let calls = calls(&log);
    assert_eq!(calls.len(), 2);
    assert!(calls[0].deadline <= Duration::from_millis(400));
    assert!(
        calls[1].deadline < calls[0].deadline,
        "the second attempt only gets what is left: {:?} then {:?}",
        calls[0].deadline,
        calls[1].deadline
    );
}

// ------------------------------------------------- the GitHub CLI resolver --

#[test]
fn the_github_cli_is_resolved_from_an_override_then_path_then_standard_directories() {
    let root = TempDir::new().unwrap();
    let base = root.path().canonicalize().unwrap();
    let on_path = base.join("path-dir");
    let standard = base.join("standard-dir");
    let named = base.join("named-dir");
    for directory in [&on_path, &standard, &named] {
        std::fs::create_dir_all(directory).unwrap();
        write_executable(&directory.join(gh_cli::EXECUTABLE), "#!/bin/sh\nexit 0\n");
    }
    let known = [standard.as_path()];
    let search = |path: &str| Some(OsString::from(path));

    // The override wins over everything, and it must be absolute.
    let explicit = named.join(gh_cli::EXECUTABLE);
    assert_eq!(
        gh_cli::resolve_within(
            Some(explicit.as_os_str()),
            search(on_path.to_str().unwrap()).as_deref(),
            &known
        ),
        Ok(explicit.clone())
    );
    assert_eq!(
        gh_cli::resolve_within(Some("gh".as_ref()), None, &known),
        Err(GhUnavailable::OverrideNotAbsolute)
    );
    assert_eq!(
        gh_cli::resolve_within(Some("./gh".as_ref()), None, &known),
        Err(GhUnavailable::OverrideNotAbsolute)
    );
    // An override that is not an executable file is refused, not searched past.
    let plain = base.join("not-executable");
    std::fs::write(&plain, "#!/bin/sh\n").unwrap();
    for candidate in [plain.as_path(), base.as_path(), &base.join("absent")] {
        assert_eq!(
            gh_cli::resolve_within(Some(candidate.as_os_str()), None, &known),
            Err(GhUnavailable::OverrideNotExecutable)
        );
    }

    // Without an override, PATH comes first, and only absolute entries count.
    assert_eq!(
        gh_cli::resolve_within(None, search(on_path.to_str().unwrap()).as_deref(), &known),
        Ok(on_path.join(gh_cli::EXECUTABLE))
    );
    let relative = format!("relative-dir:{}", on_path.display());
    assert_eq!(
        gh_cli::resolve_within(None, search(&relative).as_deref(), &known),
        Ok(on_path.join(gh_cli::EXECUTABLE))
    );
    // Then the documented standard directories.
    assert_eq!(
        gh_cli::resolve_within(None, search("").as_deref(), &known),
        Ok(standard.join(gh_cli::EXECUTABLE))
    );
    assert_eq!(
        gh_cli::resolve_within(None, None, &known),
        Ok(standard.join(gh_cli::EXECUTABLE))
    );
    // And nothing else: no shell, no install, no login, no guess.
    assert_eq!(
        gh_cli::resolve_within(None, search("").as_deref(), &[]),
        Err(GhUnavailable::NotFound)
    );
    // Every resolved path is absolute, and no message names a path.
    for unavailable in [
        GhUnavailable::NotFound,
        GhUnavailable::OverrideNotAbsolute,
        GhUnavailable::OverrideNotExecutable,
        GhUnavailable::UnsupportedPlatform,
    ] {
        let message = unavailable.to_string();
        assert!(!message.contains('/'), "{message}");
        assert!(!message.contains("auth"), "{message}");
        assert!(!message.contains("login"), "{message}");
    }
    assert!(GhUnavailable::NotFound.to_string().contains("XTRACE_GH"));
    assert!(
        gh_cli::KNOWN_DIRS
            .iter()
            .all(|dir| Path::new(dir).is_absolute())
    );
}

#[test]
fn a_missing_github_cli_is_an_actionable_refusal_that_writes_nothing() {
    let app = app(&[ALPHA]);
    let root = TempDir::new().unwrap();
    let absent = root.path().canonicalize().unwrap().join("no-such-gh");
    let (publish, events) = publisher();
    let service = PrRefreshService::production(Some(absent.into_os_string()), publish);
    let error = service
        .refresh(&app.state, &[app.id(ALPHA)])
        .expect_err("an unresolvable executable refuses the batch");
    assert!(matches!(
        error,
        RefreshError::Unavailable(GhUnavailable::OverrideNotExecutable)
    ));
    // The refusal is a typed unavailability, not a new authentication flow,
    // and it neither writes nor emits.
    let message = error.to_string();
    assert!(message.contains("XTRACE_GH"));
    assert!(!message.contains("no-such-gh"));
    assert_eq!(events.load(Ordering::SeqCst), 0);
    assert_eq!(app.stored()[&identity(ALPHA).url()].last_attempted_at, None);
}

// ------------------------------------------- the production attempt path ---

/// The production path end to end, over a `#!/bin/sh` fixture that answers
/// like `gh pr view --json`. The executable is this test's own file.
#[test]
fn the_production_path_runs_the_resolved_executable_and_records_its_answer() {
    let app = app(&[ALPHA]);
    let root = TempDir::new().unwrap();
    let base = root.path().canonicalize().unwrap();
    let recorded = base.join("argv");
    let response = serde_json::json!({
        "number": 1,
        "title": "Synthetic response",
        "url": identity(ALPHA).url(),
        "state": "MERGED",
        "mergedAt": "2026-09-19T12:34:56Z",
        "additions": 12,
        "deletions": 4,
        "headRefName": "feature/from-the-executable",
    })
    .to_string();
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"{recorded}\"\ncat <<'JSON'\n{response}\nJSON\n",
        recorded = recorded.display()
    );
    let executable = base.join("gh");
    write_executable(&executable, &script);

    let (publish, events) = publisher();
    let service = PrRefreshService::production(Some(executable.into_os_string()), publish);
    let report = service.refresh(&app.state, &[app.id(ALPHA)]).unwrap();
    assert_eq!(outcomes(&report), vec![applied()]);
    assert_eq!(events.load(Ordering::SeqCst), 1);
    // The child saw exactly the reviewed argument vector for the canonical URL.
    assert_eq!(
        std::fs::read_to_string(&recorded).unwrap().trim(),
        format!(
            "pr view {} --json number,title,url,state,mergedAt,additions,deletions,headRefName",
            identity(ALPHA).url()
        )
    );
    let row = app.stored()[&identity(ALPHA).url()].clone();
    assert_eq!(row.title.as_deref(), Some("Synthetic response"));
    assert_eq!(row.state, Some(PrState::Merged));
    assert_eq!(row.merged_at.as_deref(), Some("2026-09-19T12:34:56Z"));
    assert_eq!((row.additions, row.deletions), (Some(12), Some(4)));
    assert_eq!(
        row.head_ref_name.as_deref(),
        Some("feature/from-the-executable")
    );
}

#[test]
fn a_cancellation_during_an_attempt_stops_the_process_and_skips_the_rest() {
    let app = app(&[ALPHA, BETA, GAMMA]);
    let root = TempDir::new().unwrap();
    let base = root.path().canonicalize().unwrap();
    let executable = base.join("gh");
    // A child that never answers: only the cancellation can end this attempt.
    write_executable(&executable, "#!/bin/sh\nsleep 300\n");
    let service = Arc::new(PrRefreshService::production(
        Some(executable.into_os_string()),
        silent(),
    ));
    let running = {
        let service = Arc::clone(&service);
        let state = Arc::clone(&app.state);
        let selection = app.ids();
        std::thread::spawn(move || service.refresh(&state, &selection).unwrap())
    };
    await_active(&service);
    // Give the child a moment to exist, then cancel it from another thread.
    std::thread::sleep(Duration::from_millis(200));
    let started = Instant::now();
    assert!(service.cancel(), "a running batch is cancellable");
    let report = running.join().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the cancelled attempt returned far inside its 30s bound"
    );
    assert_eq!(
        outcomes(&report),
        vec![
            failed(PrRefreshErrorCode::Cancelled, PrRefreshWriteReport::Applied),
            skipped(PrSkipReason::Cancelled),
            skipped(PrSkipReason::Cancelled),
        ]
    );
    assert!(report.cancelled);
    // A cancelled attempt is recorded for the one it reached; the other two
    // were never attempted, so nothing about them changed.
    let stored = app.stored();
    assert_eq!(
        stored[&identity(ALPHA).url()].refresh_error,
        Some(PrRefreshError::Cancelled)
    );
    assert_eq!(stored[&identity(BETA).url()].last_attempted_at, None);
    assert_eq!(stored[&identity(GAMMA).url()].last_attempted_at, None);
    // Cancelling when nothing runs is simply false.
    assert!(!service.cancel());
}

fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

// --------------------------------------------------------- shapes on IPC ---

#[test]
fn the_reported_shapes_are_safe_and_carry_no_local_detail() {
    let app = app(&[ALPHA, BETA]);
    let log = log();
    let service = PrRefreshService::injected(
        attempts(&log, |request, index| {
            if index == 0 {
                success(request)
            } else {
                failure(request, PrRefreshError::RateLimited)
            }
        }),
        silent(),
    )
    .with_clock(StepClock::from(BASE_ATTEMPT));
    let unknown = app.ids().iter().max().copied().unwrap() + 1;
    let mut selection = app.ids();
    selection.push(unknown);
    let report = service.refresh(&app.state, &selection).unwrap();
    let listed = app.state.pr_list().unwrap();

    let json = serde_json::to_string(&report).unwrap() + &serde_json::to_string(&listed).unwrap();
    // Nothing local, nothing about a process, nothing about a credential.
    for forbidden in [
        "/Users",
        "/private",
        "/tmp",
        "xtrace.db",
        app.database.to_str().unwrap(),
        "XTRACE_GH",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "stderr",
        "argv",
        "sh -c",
    ] {
        assert!(!json.contains(forbidden), "{forbidden} leaked into {json}");
    }
    // The exact keys each shape carries.
    let report_value = serde_json::to_value(&report).unwrap();
    let mut keys: Vec<&str> = report_value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "attempted",
            "cancelled",
            "committed",
            "failed",
            "requested",
            "rows",
            "skipped",
            "succeeded",
            "unrecorded"
        ]
    );
    assert_eq!(
        report_value["rows"][0],
        serde_json::json!({
            "id": app.id(ALPHA),
            "pull_request": {
                "id": app.id(ALPHA),
                "repository": "octo-org/alpha",
                "number": 1,
                "url": identity(ALPHA).url(),
            },
            "outcome": {
                "outcome": "succeeded",
                "persistence": { "persistence": "recorded", "write": "applied" },
            },
        })
    );
    assert_eq!(
        report_value["rows"][1]["outcome"],
        serde_json::json!({
            "outcome": "failed",
            "error": "rate_limited",
            "persistence": { "persistence": "recorded", "write": "applied" },
        })
    );
    assert_eq!(
        report_value["rows"][2],
        serde_json::json!({
            "id": unknown,
            "pull_request": serde_json::Value::Null,
            "outcome": { "outcome": "skipped", "reason": "not_stored" },
        })
    );
    let listed_value = serde_json::to_value(&listed).unwrap();
    let mut row_keys: Vec<&str> = listed_value["rows"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    row_keys.sort_unstable();
    assert_eq!(
        row_keys,
        [
            "additions",
            "deletions",
            "head_ref_name",
            "last_attempted_at_ms",
            "linked_sessions",
            "merged_at",
            "pull_request",
            "refreshed_at_ms",
            "state",
            "status",
            "title"
        ]
    );
    assert_eq!(listed_value["rows"][0]["state"], "open");
    assert_eq!(
        listed_value["rows"][1]["status"],
        serde_json::json!({ "status": "failed_never_refreshed", "error": "rate_limited" })
    );
    // Every integer stays exactly representable in JSON.
    for value in [
        &listed_value["rows"][0]["pull_request"]["id"],
        &listed_value["rows"][0]["pull_request"]["number"],
        &listed_value["rows"][0]["refreshed_at_ms"],
        &listed_value["rows"][0]["last_attempted_at_ms"],
        &listed_value["rows"][0]["additions"],
    ] {
        let number = value.as_i64().expect("an exact integer");
        assert!(number.abs() < (1_i64 << 53));
    }
}

// ------------------------------------------------------------- fixtures ----

/// Fixture startup never selects live data, never resolves or runs a GitHub
/// CLI, and refreshes only its own disposable database, deterministically.
#[cfg(all(debug_assertions, feature = "fixtures"))]
#[test]
fn fixture_mode_refreshes_synthetically_over_its_disposable_database() {
    let root = TempDir::new().unwrap();
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(root.path().to_path_buf()),
            fixture: Some("F1".into()),
            // An override is present, and fixture mode must still never
            // resolve or run anything.
            github_cli: Some(OsString::from("/definitely/not/an/executable")),
            ..Default::default()
        },
        || panic!("fixture mode must not resolve the live data directory"),
        || panic!("fixture mode must not resolve the home directory"),
    )
    .unwrap();
    assert!(state.database_path().is_none());
    assert!(state.native_home().is_none());
    let now_ms = state
        .fixture_now_ms()
        .expect("fixture mode pins an instant");
    let (publish, events) = publisher();
    let service = PrRefreshService::fixture(now_ms, publish);

    // The fixture's declared links are listed, unrefreshed.
    let listed = state.pr_list().unwrap();
    assert!(!listed.rows.is_empty());
    assert!(listed.rows.iter().all(|row| {
        row.linked_sessions > 0
            && row.status == PrRefreshStatusReport::NeverAttempted
            && row.last_attempted_at_ms.is_none()
    }));
    let selection: Vec<i64> = listed.rows.iter().map(|row| row.pull_request.id).collect();
    let report = service.refresh(&state, &selection).unwrap();
    assert_eq!(report.requested, report.attempted);
    assert_eq!(report.skipped + report.unrecorded, 0);
    assert!(report.committed && !report.cancelled);
    assert_eq!(events.load(Ordering::SeqCst), 1);

    // Deterministic: every attempt is stamped with the fixture's own instant,
    // and the same identity always answers the same way.
    let refreshed = state.pr_list().unwrap();
    assert!(
        refreshed
            .rows
            .iter()
            .all(|row| row.last_attempted_at_ms == Some(now_ms))
    );
    assert!(
        refreshed
            .rows
            .iter()
            .any(|row| row.state == Some(PrStateReport::Merged))
    );
    assert!(
        refreshed
            .rows
            .iter()
            .any(|row| row.state == Some(PrStateReport::Open))
    );
    assert!(refreshed.rows.iter().any(|row| matches!(
        row.status,
        PrRefreshStatusReport::FailedNeverRefreshed { .. }
    )));
    // Replaying the same fixture refresh changes nothing at all.
    let replay = service.refresh(&state, &selection).unwrap();
    assert!(!replay.committed);
    assert_eq!(state.pr_list().unwrap(), refreshed);
    assert_eq!(events.load(Ordering::SeqCst), 1);

    assert!(matches!(
        service.refresh(&state, &[]),
        Err(RefreshError::EmptySelection)
    ));
}

// ------------------------------------------------- one storage snapshot ----

/// A pull request's metadata and its link count must come from one instant.
/// The native index writes through its own connection, so a reader that took
/// the two facts in separate statements could pair a row read before a commit
/// with a count read after it. Here a writer moves both in one transaction
/// while the list and a selection are being built, and the invariant it
/// maintains — the title says whether the pull request is linked — must hold
/// in every list the reader sees.
#[test]
fn a_concurrent_writer_is_never_observed_half_applied() {
    let app = app(&[ALPHA, BETA]);
    let beta = app.id(BETA);
    let database = app.database.clone();
    // The invariant has to hold before the writer starts too: the row is
    // linked once, so its title must already say so.
    {
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute(
                "UPDATE pull_requests SET title='linked' WHERE id=?1",
                [beta],
            )
            .unwrap();
    }
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let connection = rusqlite::Connection::open(&database).unwrap();
            connection.busy_timeout(Duration::from_secs(5)).unwrap();
            let mut linked = true;
            while !stop.load(Ordering::SeqCst) {
                let transaction = connection.unchecked_transaction().unwrap();
                if linked {
                    transaction
                        .execute("DELETE FROM pr_links WHERE pr_id=?1", [beta])
                        .unwrap();
                    transaction
                        .execute(
                            "UPDATE pull_requests SET title='unlinked' WHERE id=?1",
                            [beta],
                        )
                        .unwrap();
                } else {
                    transaction
                        .execute(
                            "INSERT INTO pr_links(session_id,pr_id,confidence,first_seen_at,last_seen_at)
                             VALUES('session-a',?1,'exact',10,20)",
                            [beta],
                        )
                        .unwrap();
                    transaction
                        .execute(
                            "UPDATE pull_requests SET title='linked' WHERE id=?1",
                            [beta],
                        )
                        .unwrap();
                }
                transaction.commit().unwrap();
                linked = !linked;
            }
        })
    };

    let mut listed_linked = 0;
    let mut listed_absent = 0;
    for _ in 0..400 {
        let listed = app.state.pr_list().unwrap();
        match listed.rows.iter().find(|row| row.pull_request.id == beta) {
            // A listed row is a linked row, so its title must say so. A split
            // read could list it with the title it had while unlinked.
            Some(row) => {
                assert_eq!(
                    row.title.as_deref(),
                    Some("linked"),
                    "listed a pull request with metadata from another instant"
                );
                assert_eq!(row.linked_sessions, 1);
                listed_linked += 1;
            }
            None => listed_absent += 1,
        }
        // The selection path reads the same snapshot: a stored ID always
        // resolves, whether or not it is still linked.
        let report =
            PrRefreshService::injected(attempts(&log(), |request, _| success(request)), silent())
                .with_clock(StepClock::from(BASE_ATTEMPT))
                .with_before_attempts(Arc::new(|cancel| cancel.cancel()))
                .refresh(&app.state, &[beta])
                .unwrap();
        assert!(
            matches!(
                report.rows[0].outcome,
                PrAttemptOutcome::Skipped {
                    reason: PrSkipReason::Cancelled | PrSkipReason::NotLinked
                }
            ),
            "a stored identifier resolved to neither linked nor unlinked: {:?}",
            report.rows[0].outcome
        );
        assert!(report.rows[0].pull_request.is_some());
    }
    stop.store(true, Ordering::SeqCst);
    writer.join().unwrap();
    // The writer really did move the row both ways while the reads ran.
    assert!(
        listed_linked > 0 && listed_absent > 0,
        "the interleaving never happened: {listed_linked} linked, {listed_absent} absent"
    );
}

// ------------------------------------------------- refusals mid-batch ------

#[test]
fn an_attempt_that_cannot_start_at_all_ends_the_batch_without_failing_anything() {
    let app = app(&[ALPHA, BETA, GAMMA]);
    let log = log();
    let (publish, events) = publisher();
    // A host condition, not this pull request's: nothing about it is stored.
    let service = PrRefreshService::injected(
        attempts(&log, |_, _| Err(GhClientError::AttemptTimeOutOfRange)),
        publish,
    );
    let selection = app.ids();
    let report = service.refresh(&app.state, &selection).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![skipped(PrSkipReason::AttemptRefused); 3]
    );
    assert_eq!((report.attempted, report.skipped), (0, 3));
    assert!(!report.committed);
    assert_eq!(
        calls(&log).len(),
        1,
        "the batch stops rather than repeating a refusal"
    );
    assert_eq!(events.load(Ordering::SeqCst), 0);
    for row in app.state.pr_list().unwrap().rows {
        assert_eq!(row.last_attempted_at_ms, None);
    }
}

#[test]
fn a_result_storage_rejects_for_one_identity_does_not_end_the_batch() {
    let app = app(&[ALPHA, BETA]);
    let (alpha, beta) = (app.id(ALPHA), app.id(BETA));
    let clock: Arc<dyn AttemptClock + Send + Sync> = Arc::new(PinnedClock(BASE_ATTEMPT));
    PrRefreshService::injected(attempts(&log(), |request, _| success(request)), silent())
        .with_clock(Arc::clone(&clock))
        .refresh(&app.state, &[alpha])
        .unwrap();

    // The same attempt time with a different answer is the storage layer's
    // equal-attempt conflict. It concerns this one identity, so the batch
    // reports it as unrecorded and goes on to the next pull request.
    let log = log();
    let (publish, events) = publisher();
    let service = PrRefreshService::injected(
        attempts(&log, |request, _| {
            let mut outcome = success(request)?;
            if let RefreshOutcome::Success(answer) = &mut outcome {
                answer.title = "A different answer".into();
            }
            Ok(outcome)
        }),
        publish,
    )
    .with_clock(clock);
    let report = service.refresh(&app.state, &[alpha, beta]).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            unrecorded(applied(), PrPersistenceError::Refused),
            applied()
        ]
    );
    // Both attempts executed; one of the two results is not stored.
    assert_eq!(
        (report.attempted, report.unrecorded, report.skipped),
        (2, 1, 0)
    );
    assert_eq!(calls(&log).len(), 2, "the batch continued");
    // The rejected result changed nothing.
    let stored = app.stored();
    assert_eq!(
        stored[&identity(ALPHA).url()].title.as_deref(),
        Some("Synthetic title 1")
    );
    assert_eq!(
        stored[&identity(BETA).url()].title.as_deref(),
        Some("A different answer")
    );
    assert_eq!(events.load(Ordering::SeqCst), 1);
}

/// The review regression: an attempt that ran is attempted, whatever storage
/// then does with its result. A selection of one keeps the two facts apart
/// with nothing else in the report to read them from.
#[test]
fn an_executed_attempt_stays_attempted_when_storage_will_not_keep_its_result() {
    // Storage refuses the result: two different answers at one attempt time.
    let app = app(&[ALPHA]);
    let alpha = app.id(ALPHA);
    let clock: Arc<dyn AttemptClock + Send + Sync> = Arc::new(PinnedClock(BASE_ATTEMPT));
    PrRefreshService::injected(attempts(&log(), |request, _| success(request)), silent())
        .with_clock(Arc::clone(&clock))
        .refresh(&app.state, &[alpha])
        .unwrap();
    let (publish, events) = publisher();
    let log = log();
    let report = PrRefreshService::injected(
        attempts(&log, |request, _| {
            let mut outcome = success(request)?;
            if let RefreshOutcome::Success(answer) = &mut outcome {
                answer.title = "A different answer".into();
            }
            Ok(outcome)
        }),
        publish,
    )
    .with_clock(clock)
    .refresh(&app.state, &[alpha])
    .unwrap();
    assert_eq!(
        outcomes(&report),
        vec![unrecorded(applied(), PrPersistenceError::Refused)]
    );
    assert_eq!(
        (
            report.requested,
            report.attempted,
            report.succeeded,
            report.failed,
            report.skipped,
            report.unrecorded
        ),
        (1, 1, 1, 0, 0, 1),
        "the attempt executed and succeeded; only its result is unstored"
    );
    assert!(!report.committed);
    assert_eq!(calls(&log).len(), 1);
    assert_eq!(events.load(Ordering::SeqCst), 0);
    // The stored row is exactly what it was before the rejected attempt, so a
    // caller can say the result was not stored rather than that nothing ran.
    assert_eq!(
        app.stored()[&identity(ALPHA).url()].title.as_deref(),
        Some("Synthetic title 1")
    );
}

/// The same regression for the other way storage can refuse a result: it
/// closed between the attempt and the write.
#[test]
fn an_executed_attempt_stays_attempted_when_storage_closed_before_the_write() {
    let closed = app(&[BETA]);
    let closing = Arc::clone(&closed.state);
    let closed_log = log();
    let (publish, events) = publisher();
    let report = PrRefreshService::injected(
        attempts(&closed_log, move |request, _| {
            closing.shutdown();
            failure(request, PrRefreshError::Timeout)
        }),
        publish,
    )
    .with_clock(StepClock::from(BASE_ATTEMPT))
    .refresh(&closed.state, &[closed.id(BETA)])
    .unwrap();
    assert_eq!(
        outcomes(&report),
        vec![unrecorded(
            failed(PrRefreshErrorCode::Timeout, PrRefreshWriteReport::Applied),
            PrPersistenceError::StorageClosed
        )]
    );
    assert_eq!(
        (
            report.attempted,
            report.succeeded,
            report.failed,
            report.skipped,
            report.unrecorded
        ),
        (1, 0, 1, 0, 1),
        "the attempt executed and failed; the batch skipped nothing"
    );
    assert!(!report.committed);
    assert_eq!(calls(&closed_log).len(), 1);
    assert_eq!(events.load(Ordering::SeqCst), 0);
    assert_eq!(
        closed.stored()[&identity(BETA).url()].last_attempted_at,
        None
    );
}

// ------------------------------------ a failing database is not a refusal --

/// A write storage *failed* (not one its rules refused) means storage cannot
/// be relied on: the executed attempt is reported unrecorded and unusable, and
/// no further request is made for the rest of the selection.
#[test]
fn a_database_failure_during_a_write_stops_the_batch_after_the_executed_attempt() {
    let app = app(&[ALPHA, BETA, GAMMA]);
    // A real SQLite failure on the write path, through the ordinary command:
    // the trigger aborts every refresh write the way a failing disk would.
    rusqlite::Connection::open(&app.database)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER failing_disk BEFORE UPDATE ON pull_requests
             BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;",
        )
        .unwrap();
    let log = log();
    let (publish, events) = publisher();
    let report = PrRefreshService::injected(attempts(&log, |request, _| success(request)), publish)
        .with_clock(StepClock::from(BASE_ATTEMPT))
        .refresh(&app.state, &app.ids())
        .unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            unrecorded(applied(), PrPersistenceError::Unusable),
            skipped(PrSkipReason::StorageUnavailable),
            skipped(PrSkipReason::StorageUnavailable),
        ]
    );
    assert_eq!(
        (report.attempted, report.unrecorded, report.skipped),
        (1, 1, 2)
    );
    assert_eq!(calls(&log).len(), 1, "no request after storage failed");
    assert!(!report.committed);
    assert_eq!(events.load(Ordering::SeqCst), 0);
}

/// The classification itself, for the failures the review named: I/O,
/// corruption and a full disk are unusable storage and stop the batch; only
/// the store's own validation vocabulary is a refusal the batch goes past.
#[test]
fn only_the_stores_own_refusal_lets_the_batch_go_on() {
    use rusqlite::ffi;
    let failures = [
        (ffi::SQLITE_IOERR, "I/O"),
        (ffi::SQLITE_CORRUPT, "corruption"),
        (ffi::SQLITE_FULL, "full disk"),
    ];
    for (code, name) in failures {
        let app = app(&[ALPHA, BETA]);
        let log = log();
        let report =
            PrRefreshService::injected(attempts(&log, |request, _| success(request)), silent())
                .with_clock(StepClock::from(BASE_ATTEMPT))
                .refresh_into(RefreshStorage {
                    targets: &|| app.state.pr_refresh_targets(&app.ids()),
                    record: &|_| {
                        Err(StateError::Store(xt_store::Error::Sqlite(
                            rusqlite::Error::SqliteFailure(ffi::Error::new(code), None),
                        )))
                    },
                })
                .unwrap();
        assert_eq!(
            outcomes(&report),
            vec![
                unrecorded(applied(), PrPersistenceError::Unusable),
                skipped(PrSkipReason::StorageUnavailable),
            ],
            "{name}"
        );
        assert_eq!(calls(&log).len(), 1, "{name}: no request after the failure");
    }

    // The store's refusal of one result is not a failure of storage.
    let app = app(&[ALPHA, BETA]);
    let log = log();
    let report =
        PrRefreshService::injected(attempts(&log, |request, _| success(request)), silent())
            .with_clock(StepClock::from(BASE_ATTEMPT))
            .refresh_into(RefreshStorage {
                targets: &|| app.state.pr_refresh_targets(&app.ids()),
                record: &|_| {
                    Err(StateError::Store(xt_store::Error::InvalidInput(
                        "an equal attempt with a different result",
                    )))
                },
            })
            .unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            unrecorded(applied(), PrPersistenceError::Refused),
            unrecorded(applied(), PrPersistenceError::Refused),
        ]
    );
    assert_eq!(calls(&log).len(), 2, "a refusal does not stop the batch");
}

/// What native fixture mode does when a pull request is refreshed again, alone
/// and overlapping a new one. The browser fixture's test
/// (`DataSource.test.ts`) asserts the very same sequence, so the preview and
/// the native fixture state agree on repeats as well as first refreshes.
#[cfg(all(debug_assertions, feature = "fixtures"))]
#[test]
fn a_repeated_fixture_refresh_is_unchanged_and_emits_nothing() {
    let root = TempDir::new().unwrap();
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(root.path().to_path_buf()),
            fixture: Some("F1".into()),
            ..Default::default()
        },
        || panic!("fixture mode must not resolve the live data directory"),
        || panic!("fixture mode must not resolve the home directory"),
    )
    .unwrap();
    let (publish, events) = publisher();
    let service = PrRefreshService::fixture(state.fixture_now_ms().unwrap(), publish);
    let ids: Vec<i64> = state
        .pr_list()
        .unwrap()
        .rows
        .iter()
        .map(|row| row.pull_request.id)
        .collect();
    // F1's first pull request succeeds (merged), its second is rate limited.
    let (first, second) = (ids[0], ids[1]);

    let once = service.refresh(&state, &[first]).unwrap();
    assert_eq!(outcomes(&once), vec![applied()]);
    assert!(once.committed);
    assert_eq!(events.load(Ordering::SeqCst), 1);

    // A partial overlap: the repeat is unchanged, the new one applies.
    let overlap = service.refresh(&state, &[first, second]).unwrap();
    assert_eq!(
        outcomes(&overlap),
        vec![
            succeeded(PrRefreshWriteReport::Unchanged),
            failed(
                PrRefreshErrorCode::RateLimited,
                PrRefreshWriteReport::Applied
            ),
        ]
    );
    assert!(overlap.committed);
    assert_eq!(events.load(Ordering::SeqCst), 2);

    // A full repeat commits nothing and emits nothing.
    let again = service.refresh(&state, &[first, second]).unwrap();
    assert_eq!(
        outcomes(&again),
        vec![
            succeeded(PrRefreshWriteReport::Unchanged),
            failed(
                PrRefreshErrorCode::RateLimited,
                PrRefreshWriteReport::Unchanged
            ),
        ]
    );
    assert_eq!((again.attempted, again.unrecorded), (2, 0));
    assert!(!again.committed);
    assert_eq!(events.load(Ordering::SeqCst), 2);
}
