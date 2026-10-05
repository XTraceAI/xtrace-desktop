//! Synthetic acceptance for the rule activity service. Every source lives in
//! a temporary home; no real home, ledger or plugin file is touched.
use super::*;
use crate::rule_activity_dto::{RuleActivityLoaded, RuleActivityPrecision};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use tauri::async_runtime::block_on;
use tempfile::TempDir;
use xt_rulebook::activity::{default_root, read_activity};

const NOW: &str = "2026-09-23T12:00:00Z";
const START: &str = "2026-09-09T12:00:00Z";
const WAIT: Duration = Duration::from_secs(10);

/// Content a row carries that must never cross the boundary, and the
/// structural fields this adapter deliberately keeps native-only.
const SECRETS: [&str; 8] = [
    "SECRET-EXCERPT-7f3a",
    "SECRET-OVERRIDE-91c2",
    "SECRET-MESSAGE-44d0",
    "SECRET-DEDUP-0b7e",
    "SECRET-AGENT-2d4c",
    "SECRET-WORKTREE-83e1",
    "SECRET-REPO-5a90",
    "SECRET-BRANCH-c1f6",
];

/// A synthetic home with a schema-2 source under its default root.
struct Home {
    home: TempDir,
}

impl Home {
    fn empty() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }

    fn with(rows: &[Value]) -> Self {
        let home = Self::empty();
        home.marker(b"2\n");
        home.ledger(&lines(rows));
        home
    }

    fn root(&self) -> PathBuf {
        default_root(self.home.path())
    }

    fn marker(&self, bytes: &[u8]) {
        std::fs::create_dir_all(self.root().join("ledger")).unwrap();
        std::fs::write(self.root().join("ledger/schema_version"), bytes).unwrap();
    }

    fn ledger(&self, bytes: &[u8]) {
        std::fs::create_dir_all(self.root().join("ledger")).unwrap();
        std::fs::write(self.root().join("ledger/fires.jsonl"), bytes).unwrap();
    }

    fn service(&self) -> RuleActivityService {
        self.service_with(Arc::new(read_activity))
    }

    fn service_with(&self, reader: Arc<Reader>) -> RuleActivityService {
        RuleActivityService::with_parts(Some(self.root()), reader, fixed_clock())
    }
}

fn fixed_clock() -> Arc<Clock> {
    Arc::new(|| NOW.parse().unwrap())
}

fn lines(rows: &[Value]) -> Vec<u8> {
    rows.iter()
        .map(|row| format!("{row}\n"))
        .collect::<String>()
        .into_bytes()
}

/// A complete schema-2 row, as the plugin writes it, carrying excluded
/// content and native-only structure full of secrets.
fn row(fire_id: &str, rule_id: &str, rulebook_id: Option<&str>, mode: &str, at: &str) -> Value {
    json!({
        "fire_id": fire_id,
        "rule_id": rule_id,
        "rulebook_id": rulebook_id,
        "rule_version": 3,
        "session_id": "0d9c7a52-native",
        "agent_id": SECRETS[4],
        "worktree": SECRETS[5],
        "host": "claude",
        "repo": SECRETS[6],
        "branch": SECRETS[7],
        "tool": "Bash",
        "hook_phase": "pre",
        "mode": mode,
        "fired_at": at,
        "excerpt": SECRETS[0],
        "override_reason": SECRETS[1],
        "source_message_id": SECRETS[2],
        "dedup_key": SECRETS[3],
        "raw_matches_before_fire": 7,
    })
}

fn loaded(result: RuleActivityResult) -> RuleActivityLoaded {
    match result {
        RuleActivityResult::Loaded(loaded) => *loaded,
        other => panic!("expected a loaded read, got {other:?}"),
    }
}

/// A reader that reports each call, then waits for a release before reading
/// the synthetic source for real, observing the service's own cancel.
struct Held {
    calls: Arc<AtomicUsize>,
    started: mpsc::Receiver<()>,
    release: mpsc::Sender<()>,
}

fn held() -> (Held, Arc<Reader>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let (started_tx, started) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let (started_tx, release_rx) = (Mutex::new(started_tx), Mutex::new(release_rx));
    let counter = Arc::clone(&calls);
    let reader: Arc<Reader> = Arc::new(move |root, limits, window, cancel| {
        counter.fetch_add(1, Ordering::SeqCst);
        started_tx.lock().unwrap().send(()).unwrap();
        release_rx.lock().unwrap().recv_timeout(WAIT).unwrap();
        read_activity(root, limits, window, cancel)
    });
    (
        Held {
            calls,
            started,
            release,
        },
        reader,
    )
}

fn counting() -> (Arc<AtomicUsize>, Arc<Reader>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let reader: Arc<Reader> = Arc::new(move |root, limits, window, cancel| {
        counter.fetch_add(1, Ordering::SeqCst);
        read_activity(root, limits, window, cancel)
    });
    (calls, reader)
}

#[test]
fn a_loaded_read_has_the_exact_wire_shape() {
    let home = Home::with(&[
        row(
            "f-1",
            "rule-a",
            Some("book-1"),
            "gate",
            "2026-09-22T10:00:00Z",
        ),
        row(
            "f-2",
            "rule-a",
            Some("book-1"),
            "advise",
            "2026-09-22T11:00:00+02:00",
        ),
        row("f-3", "rule-b", None, "shadow", "2026-09-20T08:30:00.250Z"),
        row(
            "f-4",
            "rule-a",
            Some("book-1"),
            "suppressed",
            "2026-09-01T00:00:00Z",
        ),
    ]);
    let mut wire = serde_json::to_value(home.service().read("read-1")).unwrap();
    // The reader's own clock; every other value is fixed by the source.
    assert!(wire["read_at"].as_str().unwrap().ends_with('Z'));
    wire["read_at"] = Value::Null;
    let fire = |id: &str, rule: &str, book: Value, at: &str, mode: Value| {
        json!({
            "fire_id": id, "rule_id": rule,
            "rule_version": {"kind": "number", "value": "3"},
            "rulebook_id": book, "fired_at": at, "tool": "Bash", "hook_phase": "pre",
            "host": "claude", "session_id": "0d9c7a52-native", "mode": mode,
        })
    };
    let captured = lines(&[
        row(
            "f-1",
            "rule-a",
            Some("book-1"),
            "gate",
            "2026-09-22T10:00:00Z",
        ),
        row(
            "f-2",
            "rule-a",
            Some("book-1"),
            "advise",
            "2026-09-22T11:00:00+02:00",
        ),
        row("f-3", "rule-b", None, "shadow", "2026-09-20T08:30:00.250Z"),
        row(
            "f-4",
            "rule-a",
            Some("book-1"),
            "suppressed",
            "2026-09-01T00:00:00Z",
        ),
    ])
    .len()
    .to_string();
    assert_eq!(
        wire,
        json!({
            "state": "loaded",
            "read_id": "read-1",
            "source": "default_local_rulebook",
            "schema_version": 2,
            "read_at": null,
            "window_start": START,
            "window_end": NOW,
            "counts": {
                "precision": "exact",
                "snapshot": {
                    "lines": 4, "blank_lines": 0, "valid_rows": 4, "distinct_ids": 4,
                    "duplicate_rows": 0, "conflicted_ids": 0, "conflicted_rows": 0,
                    "malformed": {"oversize_line": 0, "invalid_json": 0, "invalid_shape": 0,
                                  "invalid_timestamp": 0, "oversize_value": 0},
                },
                // f-4 is before the window: it counts in the snapshot only.
                "window_modes": {"advise": 1, "gate": 1, "suppressed": 0, "unrecognized": 1},
            },
            "coverage": {
                "captured_len": captured, "scanned_start": "0", "scanned_end": captured,
                "byte_bound_reached": false, "line_bound_reached": false,
                "leading_partial_dropped": false, "trailing_partial_dropped": false,
                "grew_after_capture": false,
                "oldest_observed": "2026-09-01T00:00:00Z",
                "newest_observed": "2026-09-22T10:00:00Z",
            },
            "latest_fires": [
                fire("f-1", "rule-a", json!("book-1"), "2026-09-22T10:00:00Z", json!({"kind": "gate"})),
                fire("f-2", "rule-a", json!("book-1"), "2026-09-22T09:00:00Z", json!({"kind": "advise"})),
                fire("f-3", "rule-b", Value::Null, "2026-09-20T08:30:00.25Z",
                     json!({"kind": "unrecognized", "value": "shadow"})),
            ],
            "fires_truncated": false,
            "observed_groups": [
                {"rulebook_id": "book-1", "rule_id": "rule-a", "latest_observed": "2026-09-22T10:00:00Z",
                 "modes": {"advise": 1, "gate": 1, "suppressed": 0, "unrecognized": 0}},
                {"rulebook_id": null, "rule_id": "rule-b", "latest_observed": "2026-09-20T08:30:00.25Z",
                 "modes": {"advise": 0, "gate": 0, "suppressed": 0, "unrecognized": 1}},
            ],
            "observed_group_count": 2,
            "groups_truncated": false,
        })
    );
}

/// The window is `[now − 14 × 24 h, now)`, anchored once at admission and
/// handed to the reader as it is reported.
#[test]
fn the_window_is_the_trailing_fourteen_days_anchored_once() {
    let home = Home::with(&[
        row("at-start", "r", None, "gate", START),
        row(
            "before-start",
            "r",
            None,
            "gate",
            "2026-09-09T11:59:59.999Z",
        ),
        row("at-end", "r", None, "gate", NOW),
    ]);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let reads = Arc::clone(&seen);
    let reader: Arc<Reader> = Arc::new(move |root, limits, window, cancel| {
        reads.lock().unwrap().push(window);
        read_activity(root, limits, window, cancel)
    });
    let ticks = Arc::new(AtomicUsize::new(0));
    let clock_ticks = Arc::clone(&ticks);
    let service = RuleActivityService::with_parts(
        Some(home.root()),
        reader,
        Arc::new(move || {
            clock_ticks.fetch_add(1, Ordering::SeqCst);
            NOW.parse().unwrap()
        }),
    );
    let read = loaded(service.read("read-1"));
    assert_eq!(
        (read.window_start.as_str(), read.window_end.as_str()),
        (START, NOW)
    );
    assert_eq!(ticks.load(Ordering::SeqCst), 1);
    let window = seen.lock().unwrap()[0].unwrap();
    assert_eq!(window.end().to_string(), NOW);
    assert_eq!(window.end().duration_since(window.start()), WINDOW);
    let ids: Vec<_> = read
        .latest_fires
        .iter()
        .map(|f| f.fire_id.as_str())
        .collect();
    assert_eq!(ids, ["at-start"]);
    assert_eq!(read.counts.snapshot.distinct_ids, 3);
}

/// How a test lays out a synthetic source before reading it.
type Setup<'a> = Box<dyn Fn(&Home) + 'a>;

/// No path, root, excluded content, native-only field or parser detail
/// reaches the wire, whatever state the source is in.
#[test]
fn nothing_but_the_whitelist_crosses_the_boundary() {
    let rows = [
        row(
            "f-1",
            "rule-a",
            Some("book-1"),
            "gate",
            "2026-09-22T10:00:00Z",
        ),
        row(
            "f-2",
            "rule-a",
            Some("book-1"),
            "gate",
            "2026-09-22T10:00:00Z",
        ),
    ];
    let assert_clean = |home: &Home, result: &RuleActivityResult| {
        let text = serde_json::to_string(result).unwrap();
        let debug = format!("{result:?}");
        for secret in SECRETS {
            assert!(!text.contains(secret) && !debug.contains(secret), "{text}");
        }
        for path in [home.home.path(), &home.root()] {
            let path = path.to_str().unwrap();
            assert!(!text.contains(path) && !debug.contains(path), "{text}");
        }
        for word in ["memhub", "fires.jsonl", "ledger/"] {
            assert!(!text.contains(word), "{word} in {text}");
        }
    };
    let home = Home::with(&rows);
    let result = home.service().read("read-1");
    assert!(matches!(result, RuleActivityResult::Loaded(_)));
    assert_clean(&home, &result);

    let cases: [(&str, Setup); 7] = [
        ("root", Box::new(|_| {})),
        (
            "ledger_dir",
            Box::new(|h| std::fs::create_dir_all(h.root()).unwrap()),
        ),
        ("schema_marker", Box::new(|h| h.ledger(&lines(&rows)))),
        (
            "ledger",
            Box::new(|h| {
                h.marker(b"2\n");
                h.ledger(&lines(&rows));
                std::fs::remove_file(h.root().join("ledger/fires.jsonl")).unwrap();
                std::os::unix::fs::symlink(
                    h.home.path().join("elsewhere.jsonl"),
                    h.root().join("ledger/fires.jsonl"),
                )
                .unwrap();
            }),
        ),
        (
            "schema_marker",
            Box::new(|h| {
                h.marker(b"{\"version\": \"SECRET-MARKER\"}");
                h.ledger(&lines(&rows));
            }),
        ),
        (
            "schema_marker",
            Box::new(|h| {
                h.marker(b"3\n");
                h.ledger(&lines(&rows));
            }),
        ),
        (
            "ledger",
            Box::new(|h| {
                h.marker(b"2\n");
            }),
        ),
    ];
    for (part, setup) in cases {
        let home = Home::empty();
        setup(&home);
        let result = home.service().read("read-2");
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["state"], "unavailable", "{wire}");
        assert_eq!(wire["part"], part, "{wire}");
        assert_eq!(wire.as_object().unwrap().len(), 5, "{wire}");
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("SECRET-MARKER")
        );
        assert_clean(&home, &result);
    }
}

/// Exact versus lower bound, and the row bound versus the group bound, stay
/// four separate facts.
#[test]
fn precision_and_both_presentation_bounds_stay_distinct() {
    // 150 fires of A, then one newer fire of B: the rows are capped, the
    // groups are not, and A keeps every one of its fires.
    let mut rows: Vec<Value> = (0..150)
        .map(|i| {
            let at = format!("2026-09-20T10:{:02}:{:02}Z", i / 60, i % 60);
            row(&format!("a-{i:03}"), "rule-a", Some("book-1"), "gate", &at)
        })
        .collect();
    rows.push(row(
        "b-0",
        "rule-b",
        Some("book-1"),
        "advise",
        "2026-09-21T00:00:00Z",
    ));
    let read = loaded(Home::with(&rows).service().read("read-1"));
    assert_eq!(read.counts.precision, RuleActivityPrecision::Exact);
    assert_eq!(read.latest_fires.len(), 100);
    assert!(read.fires_truncated && !read.groups_truncated);
    assert_eq!(read.observed_group_count, 2);
    assert_eq!(read.observed_groups[0].rule_id, "rule-b");
    assert_eq!(read.observed_groups[1].modes.gate, 150);
    assert_eq!(
        read.counts.window_modes.gate + read.counts.window_modes.advise,
        151
    );

    // 101 groups of one fire each: both bounds, still exact.
    let rows: Vec<Value> = (0..101)
        .map(|i| {
            let at = format!("2026-09-20T10:{:02}:{:02}Z", i / 60, i % 60);
            row(
                &format!("g-{i:03}"),
                &format!("rule-{i:03}"),
                None,
                "advise",
                &at,
            )
        })
        .collect();
    let read = loaded(Home::with(&rows).service().read("read-2"));
    assert_eq!(read.counts.precision, RuleActivityPrecision::Exact);
    assert_eq!(read.observed_groups.len(), 100);
    assert_eq!(read.observed_group_count, 101);
    assert!(read.fires_truncated && read.groups_truncated);

    // An unfinished last line lowers precision without any presentation bound.
    let home = Home::with(&rows[..2]);
    let mut bytes = lines(&rows[..2]);
    bytes.extend_from_slice(b"{\"fire_id\":\"half");
    home.ledger(&bytes);
    let read = loaded(home.service().read("read-3"));
    assert_eq!(read.counts.precision, RuleActivityPrecision::LowerBound);
    assert!(read.coverage.trailing_partial_dropped);
    assert!(!read.fires_truncated && !read.groups_truncated);

    // A valid empty ledger is an exact zero, not an unavailable source.
    let read = loaded(Home::with(&[]).service().read("read-4"));
    assert_eq!(read.counts.precision, RuleActivityPrecision::Exact);
    assert_eq!(read.counts.snapshot.lines, 0);
    assert!(read.latest_fires.is_empty() && read.observed_groups.is_empty());
    assert_eq!(read.coverage.oldest_observed, None);
}

#[test]
fn a_second_read_is_busy_and_reads_nothing() {
    let home = Home::with(&[row("f-1", "r", None, "gate", "2026-09-22T10:00:00Z")]);
    let (held, reader) = held();
    let service = Arc::new(home.service_with(reader));
    let first = {
        let service = Arc::clone(&service);
        std::thread::spawn(move || service.read("read-a"))
    };
    held.started.recv_timeout(WAIT).unwrap();
    assert_eq!(
        service.read("read-b"),
        RuleActivityResult::Busy {
            reason: RuleActivityBusy::AnotherRead
        }
    );
    assert_eq!(
        service.read("read-a"),
        RuleActivityResult::Busy {
            reason: RuleActivityBusy::DuplicateRead
        }
    );
    assert_eq!(held.calls.load(Ordering::SeqCst), 1);
    held.release.send(()).unwrap();
    assert_eq!(loaded(first.join().unwrap()).read_id, "read-a");
    // Once the first read has returned, the slot is free again, under any
    // name — including the one that just finished.
    held.release.send(()).unwrap();
    assert_eq!(loaded(service.read("read-a")).read_id, "read-a");
    assert_eq!(held.calls.load(Ordering::SeqCst), 2);
}

/// Admission happens before a worker exists: while one read holds the slot,
/// every other request answers at once and none waits in the pool.
#[test]
fn admission_is_never_queued_behind_the_running_read() {
    let home = Home::with(&[row("f-1", "r", None, "gate", "2026-09-22T10:00:00Z")]);
    let (held, reader) = held();
    let service = Arc::new(home.service_with(reader));
    let first = {
        let service = Arc::clone(&service);
        tauri::async_runtime::spawn(async move { read_on_worker(&service, "read-0").await })
    };
    held.started.recv_timeout(WAIT).unwrap();
    let (answered, answers) = mpsc::channel();
    for index in 1..=32 {
        let (service, answered) = (Arc::clone(&service), answered.clone());
        drop(tauri::async_runtime::spawn(async move {
            let id = format!("read-{index}");
            answered.send(read_on_worker(&service, &id).await).unwrap();
        }));
    }
    for _ in 1..=32 {
        assert_eq!(
            answers.recv_timeout(WAIT).unwrap(),
            RuleActivityResult::Busy {
                reason: RuleActivityBusy::AnotherRead
            }
        );
    }
    assert_eq!(held.calls.load(Ordering::SeqCst), 1);
    held.release.send(()).unwrap();
    assert_eq!(loaded(block_on(first).unwrap()).read_id, "read-0");
}

#[test]
fn a_cancel_before_its_read_is_remembered_once_and_reads_nothing() {
    let home = Home::with(&[]);
    let (calls, reader) = counting();
    let service = home.service_with(reader);
    service.cancel("read-1");
    service.cancel("read-1");
    assert_eq!(
        service.read("read-1"),
        RuleActivityResult::Interrupted {
            reason: RuleActivityInterruption::Cancelled
        }
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    // Remembered once: the read it named has now been answered.
    assert!(matches!(
        service.read("read-1"),
        RuleActivityResult::Loaded(_)
    ));

    // A bounded number, oldest forgotten first.
    for index in 0..REMEMBERED_CANCELS + 10 {
        service.cancel(&format!("never-{index}"));
    }
    assert_eq!(service.shared.lock().order.len(), REMEMBERED_CANCELS);
    assert_eq!(service.shared.lock().cancelled.len(), REMEMBERED_CANCELS);
    assert!(matches!(
        service.read("never-0"),
        RuleActivityResult::Loaded(_)
    ));
    let newest = format!("never-{}", REMEMBERED_CANCELS + 9);
    assert!(matches!(
        service.read(&newest),
        RuleActivityResult::Interrupted { .. }
    ));
}

/// A read refused as busy is not answered: the cancel remembered for it
/// survives the refusal and interrupts the attempt that is later admitted.
#[test]
fn a_busy_refusal_keeps_the_cancel_remembered_for_its_read() {
    let home = Home::with(&[row("f-1", "r", None, "gate", "2026-09-22T10:00:00Z")]);
    let (held, reader) = held();
    let service = Arc::new(home.service_with(reader));
    let first = {
        let service = Arc::clone(&service);
        std::thread::spawn(move || service.read("read-a"))
    };
    held.started.recv_timeout(WAIT).unwrap();
    service.cancel("read-b");
    for _ in 0..2 {
        assert_eq!(
            service.read("read-b"),
            RuleActivityResult::Busy {
                reason: RuleActivityBusy::AnotherRead
            }
        );
        assert!(service.shared.lock().cancelled.contains("read-b"));
    }
    held.release.send(()).unwrap();
    assert_eq!(loaded(first.join().unwrap()).read_id, "read-a");
    assert_eq!(
        service.read("read-b"),
        RuleActivityResult::Interrupted {
            reason: RuleActivityInterruption::Cancelled
        }
    );
    assert!(service.shared.lock().cancelled.is_empty());
    assert!(service.shared.lock().order.is_empty());
    assert_eq!(held.calls.load(Ordering::SeqCst), 1);
    // Consumed by that answer: the next attempt reads.
    held.release.send(()).unwrap();
    assert_eq!(loaded(service.read("read-b")).read_id, "read-b");
    assert_eq!(held.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn an_invalid_identifier_reads_nothing_is_not_remembered_and_is_not_echoed() {
    let home = Home::with(&[]);
    let (calls, reader) = counting();
    let service = home.service_with(reader);
    let long = "x".repeat(MAX_ID + 1);
    for id in [
        "",
        "a/b",
        "../etc",
        "open 6",
        "é",
        "<script>",
        long.as_str(),
    ] {
        service.cancel(id);
        let result = service.read(id);
        assert_eq!(result, RuleActivityResult::InvalidReadId, "{id}");
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!({"state": "invalid_read_id"})
        );
    }
    assert!(service.shared.lock().cancelled.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    // The longest valid identifier is accepted.
    assert!(matches!(
        service.read(&"x".repeat(MAX_ID)),
        RuleActivityResult::Loaded(_)
    ));
}

#[test]
fn a_cancel_during_a_read_stops_that_read_alone() {
    let home = Home::with(&[row("f-1", "r", None, "gate", "2026-09-22T10:00:00Z")]);
    let (held, reader) = held();
    let service = Arc::new(home.service_with(reader));
    let first = {
        let service = Arc::clone(&service);
        std::thread::spawn(move || service.read("read-a"))
    };
    held.started.recv_timeout(WAIT).unwrap();
    // Another name is remembered for its own read, not applied to this one.
    service.cancel("read-b");
    assert!(
        !service
            .shared
            .lock()
            .active
            .as_ref()
            .unwrap()
            .1
            .is_cancelled()
    );
    // Cancelling answers at once while the read is blocked, and the slot is
    // still held until the read returns.
    service.cancel("read-a");
    assert!(
        service
            .shared
            .lock()
            .active
            .as_ref()
            .unwrap()
            .1
            .is_cancelled()
    );
    assert_eq!(
        service.read("read-c"),
        RuleActivityResult::Busy {
            reason: RuleActivityBusy::AnotherRead
        }
    );
    held.release.send(()).unwrap();
    assert_eq!(
        first.join().unwrap(),
        RuleActivityResult::Interrupted {
            reason: RuleActivityInterruption::Cancelled
        }
    );
    assert!(matches!(
        service.read("read-b"),
        RuleActivityResult::Interrupted { .. }
    ));
    held.release.send(()).unwrap();
    assert!(matches!(
        service.read("read-c"),
        RuleActivityResult::Loaded(_)
    ));
    assert_eq!(held.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn a_cancel_after_a_read_finished_changes_no_other_read() {
    let home = Home::with(&[row("f-1", "r", None, "gate", "2026-09-22T10:00:00Z")]);
    let service = home.service();
    let first = service.read("read-a");
    assert!(matches!(first, RuleActivityResult::Loaded(_)));
    service.cancel("read-a");
    assert!(matches!(
        service.read("read-b"),
        RuleActivityResult::Loaded(_)
    ));
}

/// A reader that has read its snapshot, then waits before handing it back:
/// the window between the source read and the answer.
fn finished_then_held() -> (mpsc::Receiver<()>, mpsc::Sender<()>, Arc<Reader>) {
    let (done_tx, done) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let (done_tx, release_rx) = (Mutex::new(done_tx), Mutex::new(release_rx));
    let reader: Arc<Reader> = Arc::new(move |root, limits, window, cancel| {
        let read = read_activity(root, limits, window, cancel);
        assert!(matches!(read, ActivityRead::Snapshot(_)));
        done_tx.lock().unwrap().send(()).unwrap();
        release_rx.lock().unwrap().recv_timeout(WAIT).unwrap();
        read
    });
    (done, release, reader)
}

#[test]
fn a_cancel_or_close_after_the_source_read_discards_the_snapshot() {
    let home = Home::with(&[row("f-1", "r", None, "gate", "2026-09-22T10:00:00Z")]);
    let (done, release, reader) = finished_then_held();
    let service = Arc::new(home.service_with(reader));
    let read = {
        let service = Arc::clone(&service);
        std::thread::spawn(move || service.read("read-a"))
    };
    done.recv_timeout(WAIT).unwrap();
    service.cancel("read-a");
    release.send(()).unwrap();
    assert_eq!(
        read.join().unwrap(),
        RuleActivityResult::Interrupted {
            reason: RuleActivityInterruption::Cancelled
        }
    );

    let read = {
        let service = Arc::clone(&service);
        std::thread::spawn(move || service.read("read-b"))
    };
    done.recv_timeout(WAIT).unwrap();
    let closer = {
        let service = Arc::clone(&service);
        std::thread::spawn(move || service.close_within(WAIT))
    };
    while !service.shared.lock().closed {
        std::thread::yield_now();
    }
    release.send(()).unwrap();
    assert_eq!(read.join().unwrap(), RuleActivityResult::Closed);
    assert!(closer.join().unwrap());
}

#[test]
fn a_panicking_or_failing_read_releases_the_slot() {
    let home = Home::with(&[row("f-1", "r", None, "gate", "2026-09-22T10:00:00Z")]);
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let reader: Arc<Reader> = Arc::new(move |root, limits, window, cancel| {
        if counter.fetch_add(1, Ordering::SeqCst) < 2 {
            panic!("synthetic reader failure");
        }
        read_activity(root, limits, window, cancel)
    });
    let service = home.service_with(reader);
    // On the worker, a panic is a fixed `failed`, never its message.
    assert_eq!(
        block_on(read_on_worker(&service, "read-a")),
        RuleActivityResult::Failed
    );
    assert!(service.shared.lock().active.is_none());
    // Inline, the unwinding read still drops its slot.
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| service.read("read-b")));
    assert!(caught.is_err());
    assert!(service.shared.lock().active.is_none());
    assert!(matches!(
        service.read("read-c"),
        RuleActivityResult::Loaded(_)
    ));

    // An unavailable source releases it too.
    let missing = Home::empty();
    let service = missing.service();
    for id in ["read-d", "read-e"] {
        assert!(matches!(
            service.read(id),
            RuleActivityResult::Unavailable { .. }
        ));
    }
    assert!(service.shared.lock().active.is_none());
}

#[test]
fn closing_cancels_the_running_read_waits_for_it_and_refuses_the_rest() {
    let home = Home::with(&[row("f-1", "r", None, "gate", "2026-09-22T10:00:00Z")]);
    let (held, reader) = held();
    let service = Arc::new(home.service_with(reader));
    let first = {
        let service = Arc::clone(&service);
        std::thread::spawn(move || service.read("read-a"))
    };
    held.started.recv_timeout(WAIT).unwrap();
    let closer = {
        let service = Arc::clone(&service);
        std::thread::spawn(move || service.close_within(WAIT))
    };
    while !service.shared.lock().closed {
        std::thread::yield_now();
    }
    assert!(
        service
            .shared
            .lock()
            .active
            .as_ref()
            .unwrap()
            .1
            .is_cancelled()
    );
    assert_eq!(service.read("read-b"), RuleActivityResult::Closed);
    std::thread::sleep(Duration::from_millis(50));
    assert!(!closer.is_finished(), "close waits for the running read");
    held.release.send(()).unwrap();
    assert_eq!(first.join().unwrap(), RuleActivityResult::Closed);
    assert!(closer.join().unwrap());
    // Nothing reads after close, whatever the identifier.
    service.cancel("read-c");
    for id in ["read-a", "read-c", "read-d"] {
        assert_eq!(service.read(id), RuleActivityResult::Closed);
        assert_eq!(
            block_on(read_on_worker(&service, id)),
            RuleActivityResult::Closed
        );
    }
    assert_eq!(held.calls.load(Ordering::SeqCst), 1);
    assert!(service.close_within(Duration::ZERO));
}

#[test]
fn closing_gives_up_on_a_read_that_does_not_end_within_its_bound() {
    let home = Home::with(&[]);
    let (held, reader) = held();
    let service = Arc::new(home.service_with(reader));
    let first = {
        let service = Arc::clone(&service);
        std::thread::spawn(move || service.read("stuck"))
    };
    held.started.recv_timeout(WAIT).unwrap();
    let started = Instant::now();
    assert!(!service.close_within(Duration::from_millis(100)));
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert!(started.elapsed() < Duration::from_secs(2));
    // The blocked read still holds its slot, and nothing new is admitted.
    assert!(service.shared.lock().active.is_some());
    assert_eq!(service.read("read-b"), RuleActivityResult::Closed);
    held.release.send(()).unwrap();
    assert_eq!(first.join().unwrap(), RuleActivityResult::Closed);
    assert!(service.shared.lock().active.is_none());
}

/// Fixture mode has no native home: the answer is `not_configured`, and no
/// reader, clock or default location is consulted.
#[test]
fn no_source_is_not_configured_and_never_a_read() {
    let (calls, reader) = counting();
    let service = RuleActivityService::with_parts(
        None,
        reader,
        Arc::new(|| panic!("an unconfigured read anchors no window")),
    );
    for result in [
        service.read("read-1"),
        block_on(read_on_worker(&service, "read-2")),
        RuleActivityService::new(None).read("read-3"),
    ] {
        assert_eq!(result, RuleActivityResult::not_configured());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        RuleActivityService::new(None).read(""),
        RuleActivityResult::InvalidReadId
    );
}

/// Constructing the service only computes the default root: nothing is read,
/// checked or created, and a read of a missing source creates nothing.
#[test]
fn construction_and_a_missing_source_touch_nothing() {
    let home = Home::empty();
    let (calls, reader) = counting();
    let listing = || -> Vec<_> { walk(home.home.path()) };
    let service = home.service_with(reader);
    let native = RuleActivityService::new(Some(home.home.path()));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(native.root.as_deref(), Some(home.root().as_path()));
    assert!(listing().is_empty());
    for service in [&service, &native] {
        let wire = serde_json::to_value(service.read("read-1")).unwrap();
        assert_eq!(
            (
                wire["state"].as_str(),
                wire["part"].as_str(),
                wire["reason"].as_str()
            ),
            (Some("unavailable"), Some("root"), Some("missing"))
        );
    }
    assert!(listing().is_empty());
    // A root with no ledger directory stays as it was, too.
    std::fs::create_dir_all(home.root()).unwrap();
    let before = listing();
    assert_eq!(
        serde_json::to_value(native.read("read-2")).unwrap()["part"],
        "ledger_dir"
    );
    assert_eq!(listing(), before);
}

fn walk(directory: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(walk(&path));
        }
        found.push(path);
    }
    found.sort();
    found
}

/// The read needs no database: while another thread holds the app
/// database's lock, a read runs on a worker and answers.
#[test]
fn a_held_database_lock_does_not_delay_a_read_on_the_worker() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let state = Arc::new(
        crate::state::AppState::build(
            crate::state::StartupOptions {
                data_dir: Some(root.path().join("data")),
                native_home: Some(home.clone()),
                ..Default::default()
            },
            || panic!("default path must not be resolved"),
            || panic!("default home must not be resolved"),
        )
        .unwrap(),
    );
    let source = default_root(&home).join("ledger");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("schema_version"), b"2\n").unwrap();
    std::fs::write(
        source.join("fires.jsonl"),
        lines(&[row("f-1", "r", None, "gate", &Timestamp::now().to_string())]),
    )
    .unwrap();
    // Built as startup builds it, from the state's own native home.
    let service = Arc::new(RuleActivityService::new(state.native_home()));

    let (held, holding) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let holder = Arc::clone(&state);
    let lock = std::thread::spawn(move || {
        holder.with_store(|_| {
            held.send(()).unwrap();
            released.recv().unwrap();
            Ok(())
        })
    });
    holding.recv().unwrap();

    let ran_on = Arc::new(Mutex::new(None));
    let seen = Arc::clone(&ran_on);
    let reader: Arc<Reader> = Arc::new(move |root, limits, window, cancel| {
        *seen.lock().unwrap() = Some(std::thread::current().id());
        read_activity(root, limits, window, cancel)
    });
    let traced =
        RuleActivityService::with_parts(service.root.clone(), reader, Arc::new(Timestamp::now));
    let (answered, answer) = mpsc::channel();
    std::thread::spawn(move || {
        let issuer = std::thread::current().id();
        let result = block_on(read_on_worker(&traced, "read-1"));
        answered.send((issuer, result)).unwrap();
    });
    let (issuer, result) = answer
        .recv_timeout(WAIT)
        .expect("answers with the lock held");
    assert_eq!(loaded(result).latest_fires.len(), 1);
    let worker = ran_on.lock().unwrap().unwrap();
    assert_ne!(worker, issuer);
    assert_ne!(worker, std::thread::current().id());
    assert!(matches!(
        service.read("read-2"),
        RuleActivityResult::Loaded(_)
    ));

    release.send(()).unwrap();
    lock.join().unwrap().unwrap();
    state.shutdown();
}
