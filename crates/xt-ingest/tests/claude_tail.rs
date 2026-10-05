//! Incremental scanning and live tailing of native Claude history: the
//! watcher is registered before the initial scan, changes made during the scan
//! reach the index before ready with no later event (a session the initial
//! scan indexed and a later pass no longer saw stays in the readiness
//! report, as does a diagnostic it raised, and each keeps the report
//! incomplete; a failure the replacement of its source cannot clear stands, as
//! does a host-level failure of the initial scan),
//! live appends (the worker is idle only once every delivered change is
//! reconciled), completed
//! partial lines, new files and coalesced directory events converge within
//! seconds, truncation, replacement, restart and a failed transaction never
//! omit or duplicate a record, an appended scan reports the surface the index
//! holds and stops a record that disagrees with it, a root created after its
//! watches were decided is watched and scanned again before ready, a root the
//! platform reports removed is registered again, an event delivered before a
//! stop is honored is reconciled first, a symlinked home is followed for the fallback
//! watch, the Cursor hook's state pins are watched, sources are never modified, storage stays metadata-only, and zero-position locators from the
//! initial importer migrate to checkpoints by one full replay.
use rusqlite::Connection;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use xt_ingest::native::{
    HostStatus, ImportRequest, ProducerSource, ScanMode, SessionOutcome,
    checkpoint::{Generation, file_key},
    scan_native,
    watch::{Freshness, ProbePoint, TailEvent, Tailer, WatchConfig},
};
use xt_store::{Host, SessionSource, Store};

const A: &str = "00000000-0000-4000-8000-00000000a0a1";
const B: &str = "00000000-0000-4000-8000-00000000b0b2";
const C: &str = "00000000-0000-4000-8000-00000000c0c3";
const D: &str = "00000000-0000-4000-8000-00000000d0d4";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// One synthetic Claude record; UUIDs are unique per session and index.
fn line(index: usize, session: &str) -> String {
    let role = if index.is_multiple_of(2) {
        "user"
    } else {
        "assistant"
    };
    let tail = &session[session.len() - 4..];
    json!({
        "uuid": format!("6666{tail}-6666-4666-8666-{index:012}"),
        "type": role,
        "sessionId": session,
        "entrypoint": "cli",
        "cwd": "/repo/fixture",
        "timestamp": format!("2026-09-07T12:{:02}:{:02}Z", (index / 60) % 60, index % 60),
        "message": {"role": role, "content": [{"type": "text", "text": format!("turn {index}")}]}
    })
    .to_string()
        + "\n"
}

fn body(session: &str, range: std::ops::Range<usize>) -> String {
    range.map(|index| line(index, session)).collect()
}

struct Home {
    root: PathBuf,
    project: PathBuf,
    db: PathBuf,
}

impl Home {
    fn new(temp: &Path) -> Self {
        let root = temp.join("home");
        let project = root.join(".claude/projects/-Users-fixture");
        fs::create_dir_all(&project).unwrap();
        Self {
            root,
            project,
            db: temp.join("index.sqlite"),
        }
    }

    fn file(&self, session: &str) -> PathBuf {
        self.project.join(format!("{session}.jsonl"))
    }

    fn append(&self, session: &str, text: &str) {
        fs::OpenOptions::new()
            .append(true)
            .open(self.file(session))
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    fn store(&self) -> Store {
        Store::open(&self.db).unwrap()
    }

    fn hashes(&self) -> BTreeMap<PathBuf, String> {
        fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    let digest = Sha256::digest(fs::read(&path).unwrap());
                    out.insert(path, format!("{digest:x}"));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.root, &mut out);
        out
    }

    fn start(&self, probe: Option<xt_ingest::native::watch::Probe>) -> (Tailer, Events) {
        let events = Events::default();
        let sink = {
            let events = events.clone();
            Box::new(move |event: TailEvent| {
                events.0.lock().unwrap().push(event);
            }) as Box<dyn Fn(TailEvent) + Send>
        };
        let tailer = Tailer::start(
            self.store(),
            WatchConfig {
                home: self.root.clone(),
                hosts: vec![Host::Claude],
                producer: ProducerSource::Checkout {
                    pin: repo().join(".plugin-pin"),
                    plugin_root: None,
                },
                python: None,
                debounce: Duration::from_millis(100),
                spawn_limits: xt_ingest::native::session_creation::spawn_limits(),
                probe,
            },
            sink,
        );
        (tailer, events)
    }
}

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<TailEvent>>>);

impl Events {
    fn reconciled(&self) -> Vec<TailEvent> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, TailEvent::Reconciled { .. }))
            .cloned()
            .collect()
    }
}

/// An upper bound, not an expectation: the platform delivers events within
/// milliseconds, but a loaded machine (other test binaries, other builds)
/// can stretch a reconciliation or an event's delivery well past that.
const WAIT: Duration = Duration::from_secs(45);

/// Wait for a reconciliation newer than `after`, then for the worker to stay
/// idle with no newer reconciliation for a quiet period, so a burst the
/// platform delivered in pieces has fully settled. Returns the count seen.
fn settle(tailer: &Tailer, after: u64) -> u64 {
    assert!(
        tailer.wait_reconciled(after + 1, WAIT),
        "no reconciliation within {WAIT:?}: {:?}",
        tailer.status()
    );
    for _ in 0..90 {
        let seen = tailer.status().reconciles;
        std::thread::sleep(Duration::from_millis(400));
        let status = tailer.status();
        if status.idle && status.reconciles == seen {
            return seen;
        }
    }
    panic!("the tailer never settled: {:?}", tailer.status());
}

fn records(store: &Store, session: &str) -> usize {
    store.records(session).unwrap().len()
}

fn checkpoint(store: &Store, path: &Path) -> Option<(Generation, i64)> {
    store
        .native_checkpoint(SessionSource::Transcript, &file_key(path))
        .unwrap()
        .map(|checkpoint| {
            (
                Generation::parse(&checkpoint.generation).unwrap(),
                checkpoint.position,
            )
        })
}

fn file_len(path: &Path) -> i64 {
    fs::metadata(path).unwrap().len() as i64
}

fn assert_metadata_only(store: &Store, session: &str) {
    let rows = store.records(session).unwrap();
    assert!(!rows.is_empty());
    assert!(
        rows.iter().all(|row| row.content_json.is_none()),
        "transcript content is never stored by default"
    );
    assert_eq!(store.session(session).unwrap().unwrap().meta.title, None);
}

#[test]
fn claude_tail_watches_before_scanning_and_reconciles_changes_made_during_the_scan_with_no_later_event()
 {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..3)).unwrap();
    fs::write(home.file(B), body(B, 0..2)).unwrap();
    // While the initial scan runs, right after its first file is done: append
    // to that file and create a third session. Nothing is written afterwards.
    let fired = Arc::new(AtomicBool::new(false));
    let touched = Arc::new(Mutex::new(None::<PathBuf>));
    let probe: xt_ingest::native::watch::Probe = {
        let (fired, touched, project) = (
            Arc::clone(&fired),
            Arc::clone(&touched),
            home.project.clone(),
        );
        Arc::new(move |point: ProbePoint<'_>| {
            if let ProbePoint::FileScanned(path) = point
                && !fired.swap(true, Ordering::SeqCst)
            {
                let session = path.file_stem().unwrap().to_str().unwrap().to_owned();
                fs::OpenOptions::new()
                    .append(true)
                    .open(path)
                    .unwrap()
                    .write_all(line(7, &session).as_bytes())
                    .unwrap();
                fs::write(project.join(format!("{C}.jsonl")), body(C, 0..4)).unwrap();
                *touched.lock().unwrap() = Some(path.to_path_buf());
            }
        })
    };
    let (tailer, events) = home.start(Some(probe));
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(ready.freshness, Freshness::Live, "{ready:?}");
    assert!(fired.load(Ordering::SeqCst));
    let touched = touched.lock().unwrap().clone().unwrap();
    let touched_session = touched.file_stem().unwrap().to_str().unwrap().to_owned();
    // Everything written during the scan is indexed before ready.
    let store = home.store();
    let expected = if touched_session == A { 4 } else { 3 };
    assert_eq!(records(&store, &touched_session), expected, "{ready:?}");
    assert_eq!(records(&store, C), 4, "{ready:?}");
    assert_eq!(records(&store, A) + records(&store, B), 6);
    // The readiness report counts what the initial scan and the startup
    // reconciliation indexed together: every row of the empty index came from
    // one of them.
    for session in &ready.report.hosts[0].sessions {
        let id = session.native_session_id.as_deref().unwrap();
        assert_eq!(
            session.outcome,
            SessionOutcome::Imported {
                records_new: records(&store, id),
                records_enriched: 0
            },
            "{id}: {ready:?}"
        );
    }
    assert!(
        events.reconciled().iter().any(|event| matches!(
            event,
            TailEvent::Reconciled {
                trigger: xt_ingest::native::watch::Trigger::Startup,
                freshness: Freshness::Live,
                ..
            }
        )),
        "the startup queue was reconciled while live: {:?}",
        events.reconciled().len()
    );
    assert!(ready.report.complete(), "{ready:?}");
    assert_eq!(
        ready
            .report
            .hosts
            .iter()
            .flat_map(|host| host.sessions.iter())
            .filter(|session| session.native_session_id.as_deref() == Some(C))
            .count(),
        1,
        "the readiness report carries the session created during the scan"
    );
    // Checkpoints cover exactly the files as they stand.
    for session in [&touched_session, C] {
        let path = home.file(session);
        assert_eq!(checkpoint(&store, &path).unwrap().1, file_len(&path));
    }
    tailer.stop();
}

#[test]
fn claude_tail_ready_keeps_a_session_the_initial_scan_indexed_and_a_later_pass_no_longer_saw() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..3)).unwrap();
    fs::write(home.file(B), body(B, 0..2)).unwrap();
    // The first file scanned is removed right after its import: the change
    // queues a startup reconciliation that no longer sees it.
    let removed: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let probe: xt_ingest::native::watch::Probe = {
        let removed = Arc::clone(&removed);
        Arc::new(move |point: ProbePoint<'_>| {
            if let ProbePoint::FileScanned(path) = point {
                let mut removed = removed.lock().unwrap();
                if removed.is_none() {
                    fs::remove_file(path).unwrap();
                    *removed = Some(path.to_path_buf());
                }
            }
        })
    };
    let (tailer, events) = home.start(Some(probe));
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(ready.freshness, Freshness::Live, "{ready:?}");
    let removed = removed.lock().unwrap().clone().expect("a file was scanned");
    let gone = removed.file_stem().unwrap().to_str().unwrap().to_owned();
    assert!(
        events.reconciled().iter().any(|event| matches!(
            event,
            TailEvent::Reconciled {
                trigger: xt_ingest::native::watch::Trigger::Startup,
                ..
            }
        )),
        "the removal was reconciled before ready: {ready:?}"
    );
    // History is kept, and the readiness report still names the removed
    // session with the records the initial scan indexed.
    let store = home.store();
    assert_eq!(records(&store, A), 3);
    assert_eq!(records(&store, B), 2);
    let sessions = &ready.report.hosts[0].sessions;
    for id in [A, B] {
        let session = sessions
            .iter()
            .find(|session| session.native_session_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("{id} is missing from the readiness report: {ready:?}"));
        assert_eq!(
            session.outcome,
            SessionOutcome::Imported {
                records_new: records(&store, id),
                records_enriched: 0
            },
            "{id}: {ready:?}"
        );
    }
    assert!(
        sessions
            .iter()
            .any(|s| s.native_session_id.as_deref() == Some(gone.as_str()))
    );
    tailer.stop();
}

#[test]
fn claude_tail_ready_stays_incomplete_around_a_retained_session_that_was_not_imported() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    // A carries a record without a UUID, which the scan drops: partial.
    let mut broken: serde_json::Value = serde_json::from_str(line(2, A).trim()).unwrap();
    broken.as_object_mut().unwrap().remove("uuid");
    fs::write(home.file(A), body(A, 0..2) + &broken.to_string() + "\n").unwrap();
    fs::write(home.file(B), body(B, 0..2)).unwrap();
    // A is removed right after its import, so the startup reconciliation
    // that its removal queues no longer sees it and reports complete.
    let probe: xt_ingest::native::watch::Probe = {
        let file = home.file(A);
        Arc::new(move |point: ProbePoint<'_>| {
            if let ProbePoint::FileScanned(path) = point
                && path == file
            {
                fs::remove_file(path).unwrap();
            }
        })
    };
    let (tailer, events) = home.start(Some(probe));
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(ready.freshness, Freshness::Live, "{ready:?}");
    assert!(
        events.reconciled().iter().any(|event| matches!(
            event,
            TailEvent::Reconciled {
                trigger: xt_ingest::native::watch::Trigger::Startup,
                ..
            }
        )),
        "the removal was reconciled before ready: {ready:?}"
    );
    let store = home.store();
    assert_eq!(records(&store, A), 2);
    assert_eq!(records(&store, B), 2);
    // The retained partial session keeps the host, and the report, incomplete.
    let claude = &ready.report.hosts[0];
    assert_eq!(claude.status, HostStatus::Incomplete, "{ready:?}");
    assert!(!ready.report.complete());
    let outcome = |id: &str| {
        claude
            .sessions
            .iter()
            .find(|session| session.native_session_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("{id} is missing from the readiness report: {ready:?}"))
            .outcome
            .clone()
    };
    assert!(
        matches!(
            outcome(A),
            SessionOutcome::Partial {
                records_new: 2,
                records_dropped: 1,
                ..
            }
        ),
        "{ready:?}"
    );
    assert_eq!(
        outcome(B),
        SessionOutcome::Imported {
            records_new: 2,
            records_enriched: 0
        }
    );
    tailer.stop();
}

#[cfg(unix)]
#[test]
fn claude_tail_ready_keeps_a_diagnostic_the_later_pass_no_longer_saw() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..2)).unwrap();
    // A project directory the scan cannot read: a discovery diagnostic and
    // no session for the transcript below it.
    let locked = home.root.join(".claude/projects/-Users-locked");
    fs::create_dir_all(&locked).unwrap();
    fs::write(locked.join(format!("{C}.jsonl")), body(C, 0..1)).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    // Right after A is scanned the directory is removed (made readable
    // first) and recreated with another transcript, so the startup
    // reconciliation its change queues imports that one below the same path
    // and reports complete: the transcript the diagnostic hid is gone unread.
    let probe: xt_ingest::native::watch::Probe = {
        let locked = locked.clone();
        let replaced = AtomicBool::new(false);
        Arc::new(move |point: ProbePoint<'_>| {
            if matches!(point, ProbePoint::FileScanned(_)) && !replaced.swap(true, Ordering::SeqCst)
            {
                fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
                fs::remove_dir_all(&locked).unwrap();
                fs::create_dir_all(&locked).unwrap();
                fs::write(locked.join(format!("{D}.jsonl")), body(D, 0..1)).unwrap();
            }
        })
    };
    let (tailer, events) = home.start(Some(probe));
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(ready.freshness, Freshness::Live, "{ready:?}");
    assert!(
        events.reconciled().iter().any(|event| matches!(
            event,
            TailEvent::Reconciled {
                trigger: xt_ingest::native::watch::Trigger::Startup,
                ..
            }
        )),
        "the removal was reconciled before ready: {ready:?}"
    );
    let store = home.store();
    assert_eq!(records(&store, A), 2);
    assert_eq!(records(&store, D), 1, "the replacement was imported");
    assert_eq!(
        records(&store, C),
        0,
        "the locked transcript was never read"
    );
    // The diagnostic stays (what was imported below its path is not the
    // source it hid), and keeps the host and the report incomplete.
    let claude = &ready.report.hosts[0];
    assert_eq!(claude.status, HostStatus::Incomplete, "{ready:?}");
    assert!(!ready.report.complete());
    assert!(
        claude
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.path.as_deref() == Some(&*locked.to_string_lossy())),
        "{ready:?}"
    );
    tailer.stop();
}

#[test]
fn claude_tail_ready_keeps_a_failure_the_replacement_of_its_source_cannot_clear() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    // A carries a record without a UUID, which the scan drops: partial.
    let mut broken: serde_json::Value = serde_json::from_str(line(2, A).trim()).unwrap();
    broken.as_object_mut().unwrap().remove("uuid");
    fs::write(home.file(A), body(A, 0..2) + &broken.to_string() + "\n").unwrap();
    // Right after its scan A is replaced by a clean transcript: the startup
    // reconciliation imports the replacement fully, but what the failure
    // left unread in the original is gone.
    let probe: xt_ingest::native::watch::Probe = {
        let file = home.file(A);
        let replaced = AtomicBool::new(false);
        Arc::new(move |point: ProbePoint<'_>| {
            if matches!(point, ProbePoint::FileScanned(_)) && !replaced.swap(true, Ordering::SeqCst)
            {
                fs::write(&file, body(A, 0..3)).unwrap();
            }
        })
    };
    let (tailer, events) = home.start(Some(probe));
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(ready.freshness, Freshness::Live, "{ready:?}");
    assert!(
        events.reconciled().iter().any(|event| matches!(
            event,
            TailEvent::Reconciled {
                trigger: xt_ingest::native::watch::Trigger::Startup,
                ..
            }
        )),
        "the replacement was reconciled before ready: {ready:?}"
    );
    assert_eq!(records(&home.store(), A), 3, "the replacement is indexed");
    // The failure stands in the readiness report, with the records both
    // passes indexed counted, and keeps the host and the report incomplete.
    let claude = &ready.report.hosts[0];
    assert_eq!(claude.status, HostStatus::Incomplete, "{ready:?}");
    assert!(!ready.report.complete());
    assert!(
        matches!(
            claude.sessions[0].outcome,
            SessionOutcome::Partial {
                records_new: 3,
                records_dropped: 1,
                ..
            }
        ),
        "{ready:?}"
    );
    tailer.stop();
}

#[test]
fn claude_tail_is_idle_only_once_a_change_delivered_during_a_reconciliation_is_indexed() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..2)).unwrap();
    // During the first live reconciliation a new transcript is written and
    // its event delivered before that reconciliation ends.
    let probe: xt_ingest::native::watch::Probe = {
        let file = home.file(B);
        let written = AtomicBool::new(false);
        Arc::new(move |point: ProbePoint<'_>| {
            if matches!(point, ProbePoint::Reconciling(_))
                && file
                    .parent()
                    .unwrap()
                    .join(format!("{A}.jsonl"))
                    .metadata()
                    .unwrap()
                    .len()
                    > body(A, 0..2).len() as u64
                && !written.swap(true, Ordering::SeqCst)
            {
                fs::write(&file, body(B, 0..3)).unwrap();
                std::thread::sleep(Duration::from_millis(600));
            }
        })
    };
    let (tailer, _events) = home.start(Some(probe));
    tailer.wait_ready(WAIT).expect("ready");
    let seen = tailer.status().reconciles;
    home.append(A, &line(2, A));
    // Idle means every delivered change was reconciled: when the waiter
    // wakes for the first reconciliation, the second is done as well.
    assert!(
        tailer.wait_reconciled(seen + 1, WAIT),
        "{:?}",
        tailer.status()
    );
    let status = tailer.status();
    assert!(status.reconciles >= seen + 2, "{status:?}");
    let store = home.store();
    assert_eq!(records(&store, A), 3);
    assert_eq!(records(&store, B), 3);
    tailer.stop();
}

#[test]
fn claude_tail_ready_keeps_a_host_level_failure_a_later_pass_cannot_clear() {
    // The projects root is a file when the tailer starts: the initial scan
    // cannot read the host at all (no session, no diagnostic, a status).
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::remove_dir_all(&home.project).unwrap();
    let projects = home.root.join(".claude/projects");
    fs::remove_dir_all(&projects).unwrap();
    fs::write(&projects, b"not a directory").unwrap();
    // Once the initial scan is done the root becomes a real directory with a
    // transcript, before the startup reconciliation that its change queues.
    let probe: xt_ingest::native::watch::Probe = {
        let projects = projects.clone();
        let project = home.project.clone();
        let file = home.file(A);
        Arc::new(move |point: ProbePoint<'_>| {
            if matches!(point, ProbePoint::InitialScanDone) {
                fs::remove_file(&projects).unwrap();
                fs::create_dir_all(&project).unwrap();
                fs::write(&file, body(A, 0..2)).unwrap();
            }
        })
    };
    let (tailer, events) = home.start(Some(probe));
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(ready.freshness, Freshness::Live, "{ready:?}");
    assert!(
        events.reconciled().iter().any(|event| matches!(
            event,
            TailEvent::Reconciled {
                trigger: xt_ingest::native::watch::Trigger::Startup,
                ..
            }
        )),
        "the new root was reconciled before ready: {ready:?}"
    );
    assert_eq!(records(&home.store(), A), 2, "the transcript is indexed");
    // The host the initial scan could not read stays incomplete in the
    // readiness report, its reason kept, although the later pass completed.
    let claude = &ready.report.hosts[0];
    assert_eq!(claude.status, HostStatus::Incomplete, "{ready:?}");
    assert!(!ready.report.complete());
    assert!(
        claude
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("not a directory")),
        "{ready:?}"
    );
    tailer.stop();
}

#[test]
fn claude_tail_imports_appends_completed_partial_lines_new_files_and_directories_within_seconds() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..2)).unwrap();
    let before = home.hashes();
    let (tailer, _events) = home.start(None);
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(ready.freshness, Freshness::Live);
    let store = home.store();
    assert_eq!(records(&store, A), 2);
    let mut seen = tailer.status().reconciles;

    // A complete record appended to a scanned file.
    let started = std::time::Instant::now();
    home.append(A, &line(2, A));
    seen = settle(&tailer, seen);
    assert_eq!(records(&store, A), 3);
    assert!(
        started.elapsed() < Duration::from_secs(2) + Duration::from_millis(1600),
        "settled in {:?}",
        started.elapsed()
    );

    // A partial line waits; its completion is imported once.
    let partial = line(3, A);
    let (head, rest) = partial.split_at(partial.len() / 2);
    home.append(A, head);
    seen = settle(&tailer, seen);
    assert_eq!(records(&store, A), 3, "a partial line is not a record");
    assert_eq!(
        checkpoint(&store, &home.file(A)).unwrap().1,
        file_len(&home.file(A)) - head.len() as i64,
        "the checkpoint stops before the partial line"
    );
    home.append(A, rest);
    seen = settle(&tailer, seen);
    assert_eq!(records(&store, A), 4);
    assert_eq!(
        checkpoint(&store, &home.file(A)).unwrap().1,
        file_len(&home.file(A))
    );

    // Duplicate events for unchanged content add nothing.
    for _ in 0..3 {
        let current = fs::read(home.file(A)).unwrap();
        fs::write(home.file(A), current).unwrap();
    }
    seen = settle(&tailer, seen);
    assert_eq!(records(&store, A), 4);
    let unique: std::collections::BTreeSet<_> = store
        .records(A)
        .unwrap()
        .iter()
        .map(|row| row.uuid.clone())
        .collect();
    assert_eq!(unique.len(), 4);

    // A new file in a known project, and a whole new project directory with
    // two files created at once (a directory-level, coalesced event).
    fs::write(home.file(B), body(B, 0..3)).unwrap();
    let other = home.root.join(".claude/projects/-Users-other");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join(format!("{C}.jsonl")), body(C, 0..2)).unwrap();
    fs::write(other.join(format!("{D}.jsonl")), body(D, 0..5)).unwrap();
    settle(&tailer, seen);
    assert_eq!(records(&store, B), 3);
    assert_eq!(records(&store, C), 2);
    assert_eq!(records(&store, D), 5);
    assert_metadata_only(&store, A);
    assert_metadata_only(&store, D);
    tailer.stop();
    // Sources changed only where this test wrote them, and never by the tailer.
    let after = home.hashes();
    assert_eq!(before.len(), 1);
    assert_eq!(after.len(), 4);
    assert_eq!(fs::read_to_string(home.file(A)).unwrap(), body(A, 0..4));
}

#[test]
fn claude_tail_converges_after_truncation_and_replacement_without_duplicates() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..5)).unwrap();
    let (tailer, _events) = home.start(None);
    tailer.wait_ready(WAIT).expect("ready");
    let store = home.store();
    let path = home.file(A);
    let (first_generation, first_position) = checkpoint(&store, &path).unwrap();
    assert_eq!(first_position, file_len(&path));
    let mut seen = tailer.status().reconciles;

    // In-place truncation to two lines, then growth beyond the old length.
    fs::write(&path, body(A, 0..2)).unwrap();
    seen = settle(&tailer, seen);
    assert_eq!(
        records(&store, A),
        5,
        "history is kept when a source shrinks"
    );
    let (truncated_generation, truncated_position) = checkpoint(&store, &path).unwrap();
    assert_eq!(truncated_position, file_len(&path));
    assert_eq!(
        (&first_generation, &truncated_generation),
        (&first_generation, &truncated_generation)
    );
    fs::write(&path, body(A, 0..7)).unwrap();
    seen = settle(&tailer, seen);
    assert_eq!(records(&store, A), 7);
    assert_eq!(checkpoint(&store, &path).unwrap().1, file_len(&path));

    // Atomic replacement (another inode) with different, shorter content.
    let staging = home.project.join("staging.tmp");
    fs::write(&staging, body(A, 7..9)).unwrap();
    fs::rename(&staging, &path).unwrap();
    seen = settle(&tailer, seen);
    assert_eq!(
        records(&store, A),
        9,
        "the replacement's records are imported once"
    );
    let (replaced_generation, replaced_position) = checkpoint(&store, &path).unwrap();
    assert_eq!(replaced_position, file_len(&path));
    let inode = |generation: &Generation| {
        let Generation::File { ino, .. } = generation;
        *ino
    };
    assert_ne!(
        inode(&replaced_generation),
        inode(&first_generation),
        "a replacement is a new generation"
    );

    // An in-place rewrite of the same length: the digest disagrees, the file
    // is read whole again, and nothing duplicates.
    let mut rewritten = body(A, 9..11);
    rewritten.truncate(rewritten.len());
    fs::write(&path, &rewritten).unwrap();
    settle(&tailer, seen);
    assert_eq!(records(&store, A), 11);
    let unique: std::collections::BTreeSet<_> = store
        .records(A)
        .unwrap()
        .iter()
        .map(|row| row.uuid.clone())
        .collect();
    assert_eq!(unique.len(), 11);
    tailer.stop();
}

#[test]
fn claude_tail_restart_resumes_from_checkpoints_without_rereading_or_duplicating() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..3)).unwrap();
    fs::write(home.file(B), body(B, 0..2)).unwrap();
    let (tailer, _events) = home.start(None);
    tailer.wait_ready(WAIT).expect("ready");
    // The initial scan counts as the first reconciliation.
    assert!(tailer.status().reconciles >= 1, "{:?}", tailer.status());
    assert!(tailer.wait_reconciled(1, Duration::from_millis(50)));
    tailer.stop();
    // Offline changes: one file grows, one is untouched, one is new.
    home.append(A, &line(3, A));
    fs::write(home.file(C), body(C, 0..1)).unwrap();
    let (tailer, _events) = home.start(None);
    let ready = tailer.wait_ready(WAIT).expect("ready");
    let claude = &ready.report.hosts[0];
    assert_eq!(claude.status, HostStatus::Complete, "{ready:?}");
    let outcome = |session: &str| {
        claude
            .sessions
            .iter()
            .find(|result| result.native_session_id.as_deref() == Some(session))
            .unwrap()
            .outcome
            .clone()
    };
    assert_eq!(
        outcome(A),
        SessionOutcome::Imported {
            records_new: 1,
            records_enriched: 0
        },
        "only the appended record is new after a restart"
    );
    assert_eq!(
        outcome(B),
        SessionOutcome::Imported {
            records_new: 0,
            records_enriched: 0
        }
    );
    assert_eq!(
        claude
            .sessions
            .iter()
            .find(|result| result.native_session_id.as_deref() == Some(B))
            .unwrap()
            .source_surface
            .as_deref(),
        Some("cli"),
        "an untouched file still reports its stored surface"
    );
    assert_eq!(
        outcome(C),
        SessionOutcome::Imported {
            records_new: 1,
            records_enriched: 0
        }
    );
    let store = home.store();
    assert_eq!(records(&store, A), 4);
    assert_eq!(records(&store, B), 2);
    assert_eq!(records(&store, C), 1);
    tailer.stop();
}

#[test]
fn claude_tail_keeps_progress_behind_a_failed_transaction_and_converges_on_retry() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..2)).unwrap();
    let (tailer, events) = home.start(None);
    tailer.wait_ready(WAIT).expect("ready");
    let store = home.store();
    let path = home.file(A);
    let before = checkpoint(&store, &path).unwrap();
    let mut seen = tailer.status().reconciles;
    // Every record insert fails until the trigger is dropped.
    let connection = Connection::open(&home.db).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER inject_failure BEFORE INSERT ON records BEGIN
                 SELECT RAISE(ABORT, 'synthetic transaction failure'); END;",
        )
        .unwrap();
    home.append(A, &line(2, A));
    seen = settle(&tailer, seen);
    assert_eq!(records(&store, A), 2, "the failed batch stored nothing");
    assert_eq!(
        checkpoint(&store, &path).unwrap(),
        before,
        "progress did not advance past the unimported input"
    );
    let failed = events.reconciled();
    let last = failed.last().unwrap();
    let TailEvent::Reconciled { report, .. } = last else {
        unreachable!()
    };
    assert!(!report.complete(), "the failure is reported: {report:?}");
    assert!(
        matches!(
            &report.hosts[0].sessions[0].outcome,
            SessionOutcome::Skipped { reason } if reason.contains("could not be written")
        ),
        "{report:?}"
    );
    connection
        .execute_batch("DROP TRIGGER inject_failure")
        .unwrap();
    // The next change re-reads from the kept checkpoint: both records land once.
    home.append(A, &line(3, A));
    settle(&tailer, seen);
    assert_eq!(records(&store, A), 4);
    assert_eq!(checkpoint(&store, &path).unwrap().1, file_len(&path));
    tailer.stop();
}

#[test]
fn claude_tail_migrates_zero_position_locators_by_one_full_replay() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..3)).unwrap();
    let path = home.file(A);
    // An index left by the initial importer: rows, a zero-position locator,
    // and no checkpoint.
    let mut store = home.store();
    let request = ImportRequest {
        home: &home.root,
        hosts: &[Host::Claude],
        producer: &ProducerSource::Checkout {
            pin: repo().join(".plugin-pin"),
            plugin_root: None,
        },
        python: None,
        observed_at: 1,
        cancel: None,
    };
    assert!(scan_native(&mut store, &request, ScanMode::Replay).complete());
    store
        .clear_native_checkpoint(SessionSource::Transcript, &file_key(&path))
        .unwrap();
    let locator = store
        .source_cursor(SessionSource::Transcript, &file_key(&path))
        .unwrap()
        .unwrap();
    assert_eq!(locator.position, 0);
    assert!(checkpoint(&store, &path).is_none());
    // The first resuming scan reads the whole file (the locator proves
    // nothing), adds no rows, and leaves a checkpoint the next scan can prove.
    let report = scan_native(&mut store, &request, ScanMode::Resume);
    assert!(report.complete(), "{report:?}");
    assert_eq!(
        report.hosts[0].sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 0,
            records_enriched: 0
        }
    );
    assert_eq!(records(&store, A), 3);
    let (generation, position) = checkpoint(&store, &path).unwrap();
    assert_eq!(position, file_len(&path));
    assert!(
        matches!(generation, Generation::File { lines: 3, .. }),
        "{generation:?}"
    );
    assert_eq!(
        store
            .source_cursor(SessionSource::Transcript, &file_key(&path))
            .unwrap()
            .unwrap()
            .position,
        0,
        "the locator stays a zero-position locator"
    );
    // Proven unchanged: the next scan touches nothing, and still reports the
    // session's stored surface.
    let report = scan_native(&mut store, &request, ScanMode::Resume);
    assert!(report.complete());
    assert_eq!(records(&store, A), 3);
    assert_eq!(
        report.hosts[0].sessions[0].source_surface.as_deref(),
        Some("cli"),
        "an unchanged file reports the surface the index holds"
    );
    // A replay rereads everything and still adds nothing.
    let report = scan_native(&mut store, &request, ScanMode::Replay);
    assert!(report.complete());
    assert_eq!(records(&store, A), 3);
}

#[test]
fn claude_tail_appended_records_report_and_must_repeat_the_stored_surface() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..3)).unwrap();
    let path = home.file(A);
    let mut store = home.store();
    let request = ImportRequest {
        home: &home.root,
        hosts: &[Host::Claude],
        producer: &ProducerSource::Checkout {
            pin: repo().join(".plugin-pin"),
            plugin_root: None,
        },
        python: None,
        observed_at: 1,
        cancel: None,
    };
    assert!(scan_native(&mut store, &request, ScanMode::Resume).complete());
    assert_eq!(
        store.session(A).unwrap().unwrap().meta.surface.as_deref(),
        Some("cli")
    );
    // A record naming no surface, appended behind the checkpoint: the scan
    // reads only the suffix, and still reports the surface the prefix settled
    // and the index holds.
    let mut unlabeled: serde_json::Value = serde_json::from_str(line(3, A).trim()).unwrap();
    assert!(
        unlabeled
            .as_object_mut()
            .unwrap()
            .remove("entrypoint")
            .is_some()
    );
    home.append(A, &(unlabeled.to_string() + "\n"));
    let report = scan_native(&mut store, &request, ScanMode::Resume);
    assert!(report.complete(), "{report:?}");
    let session = &report.hosts[0].sessions[0];
    assert_eq!(
        session.outcome,
        SessionOutcome::Imported {
            records_new: 1,
            records_enriched: 0
        }
    );
    assert_eq!(
        session.source_surface.as_deref(),
        Some("cli"),
        "an appended scan reports the surface the index holds"
    );
    assert_eq!(records(&store, A), 4);
    assert_eq!(
        store.session(A).unwrap().unwrap().meta.surface.as_deref(),
        Some("cli")
    );
    let held = checkpoint(&store, &path).unwrap();
    assert_eq!(held.1, file_len(&path));
    // A record on another surface stops the file at that line, as it would
    // in a whole-file read: nothing is written and the checkpoint stays.
    let mut other: serde_json::Value = serde_json::from_str(line(4, A).trim()).unwrap();
    other["entrypoint"] = json!("sdk");
    home.append(A, &(other.to_string() + "\n"));
    let report = scan_native(&mut store, &request, ScanMode::Resume);
    assert!(!report.complete(), "{report:?}");
    let session = &report.hosts[0].sessions[0];
    match &session.outcome {
        SessionOutcome::Skipped { reason } => assert!(
            reason.contains("record surface disagrees with the file's surface")
                && reason.contains("stream line 5"),
            "{reason}"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(session.source_surface.as_deref(), Some("cli"));
    assert_eq!(records(&store, A), 4);
    assert_eq!(checkpoint(&store, &path).unwrap(), held);
    // The line corrected to the file's surface imports behind the same
    // checkpoint.
    other["entrypoint"] = json!("cli");
    let mut kept = fs::read(&path).unwrap();
    kept.truncate(held.1 as usize);
    kept.extend((other.to_string() + "\n").into_bytes());
    fs::write(&path, kept).unwrap();
    let report = scan_native(&mut store, &request, ScanMode::Resume);
    assert!(report.complete(), "{report:?}");
    assert_eq!(
        report.hosts[0].sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 1,
            records_enriched: 0
        }
    );
    assert_eq!(records(&store, A), 5);
    assert_eq!(checkpoint(&store, &path).unwrap().1, file_len(&path));
    // The file rewritten whole with records naming no surface (a new
    // generation, read from the beginning), and a replay: both still report
    // the surface the index holds.
    let unlabeled = (0..5)
        .map(|index| {
            let mut record: serde_json::Value =
                serde_json::from_str(line(index, A).trim()).unwrap();
            record.as_object_mut().unwrap().remove("entrypoint");
            record.to_string() + "\n"
        })
        .collect::<String>();
    fs::write(&path, unlabeled).unwrap();
    for mode in [ScanMode::Resume, ScanMode::Replay] {
        let report = scan_native(&mut store, &request, mode);
        assert!(report.complete(), "{report:?}");
        assert_eq!(
            report.hosts[0].sessions[0].source_surface.as_deref(),
            Some("cli"),
            "a whole read of a known session reports the surface the index holds: {report:?}"
        );
    }
    assert_eq!(records(&store, A), 5);
}

#[test]
fn claude_tail_reports_a_watcher_failure_as_degraded_never_silent_ready() {
    let temp = tempfile::TempDir::new().unwrap();
    let absent = temp.path().join("absent-home");
    let events = Events::default();
    let sink = {
        let events = events.clone();
        Box::new(move |event: TailEvent| events.0.lock().unwrap().push(event))
            as Box<dyn Fn(TailEvent) + Send>
    };
    let tailer = Tailer::start(
        Store::open(temp.path().join("index.sqlite")).unwrap(),
        WatchConfig {
            home: absent.clone(),
            hosts: vec![Host::Claude],
            producer: ProducerSource::Checkout {
                pin: repo().join(".plugin-pin"),
                plugin_root: None,
            },
            python: None,
            debounce: Duration::from_millis(100),
            spawn_limits: xt_ingest::native::session_creation::spawn_limits(),
            probe: None,
        },
        sink,
    );
    let ready = tailer.wait_ready(WAIT).expect("ready");
    match &ready.freshness {
        Freshness::Degraded { reason } => {
            assert!(reason.contains("could not be watched"), "{reason}")
        }
        Freshness::Live => panic!("a watcher that failed must not read as live"),
    }
    assert_eq!(ready.report.hosts[0].status, HostStatus::MissingSource);
    let status = tailer.status();
    assert!(status.watched.is_empty(), "{status:?}");
    assert!(status.last_error.is_some());
    tailer.stop();
    assert!(
        events.0.lock().unwrap().iter().any(|event| matches!(
            event,
            TailEvent::Stopped {
                freshness: Freshness::Degraded { .. }
            }
        )),
        "the final event carries the degraded freshness"
    );
}

#[test]
fn claude_tail_watches_a_root_that_appears_later_before_enumerating_it() {
    // No Claude root at all when the tailer starts: only the home is watched.
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::remove_dir_all(home.root.join(".claude")).unwrap();
    let (tailer, _events) = home.start(None);
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(ready.freshness, Freshness::Live);
    assert_eq!(ready.report.hosts[0].status, HostStatus::MissingSource);
    assert_eq!(
        tailer.status().watched,
        vec![
            home.root
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        ]
    );
    let seen = tailer.status().reconciles;
    // The root appears with a first session: the home watch sees it, the new
    // root is watched before it is enumerated, and the session is imported.
    fs::create_dir_all(&home.project).unwrap();
    fs::write(home.file(A), body(A, 0..2)).unwrap();
    let seen = settle(&tailer, seen);
    let store = home.store();
    assert_eq!(records(&store, A), 2);
    let watched = tailer.status().watched;
    assert!(
        watched.contains(
            &home
                .root
                .join(".claude/projects")
                .to_string_lossy()
                .into_owned()
        ),
        "{watched:?}"
    );
    // A later file below the new root is seen through its own recursive watch.
    fs::write(home.file(B), body(B, 0..3)).unwrap();
    settle(&tailer, seen);
    assert_eq!(records(&store, B), 3);
    tailer.stop();
}

#[test]
fn claude_tail_watches_a_root_created_after_its_watches_were_decided_before_ready() {
    // No Claude root when the tailer starts. The root appears after the pass
    // that decided to watch only the home and before that watch exists, so
    // its creation is never reported; the initial scan enumerates it all the
    // same. It must be watched, and scanned again, before ready.
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::remove_dir_all(home.root.join(".claude")).unwrap();
    let created = Arc::new(AtomicBool::new(false));
    let probe: xt_ingest::native::watch::Probe = {
        let created = Arc::clone(&created);
        let project = home.project.clone();
        let file = home.file(A);
        Arc::new(move |point: ProbePoint<'_>| {
            if matches!(point, ProbePoint::WatchesDecided) && !created.swap(true, Ordering::SeqCst)
            {
                fs::create_dir_all(&project).unwrap();
                fs::write(&file, body(A, 0..2)).unwrap();
            }
        })
    };
    let (tailer, _events) = home.start(Some(probe));
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(ready.freshness, Freshness::Live, "{ready:?}");
    assert!(created.load(Ordering::SeqCst));
    let store = home.store();
    assert_eq!(records(&store, A), 2, "{ready:?}");
    let watched = tailer.status().watched;
    assert!(
        watched.contains(
            &home
                .root
                .join(".claude/projects")
                .to_string_lossy()
                .into_owned()
        ),
        "the root is watched before ready: {watched:?}"
    );
    // A change below the root is seen through its own recursive watch.
    let seen = tailer.status().reconciles;
    home.append(A, &line(2, A));
    settle(&tailer, seen);
    assert_eq!(records(&store, A), 3);
    tailer.stop();
}

#[test]
fn claude_tail_watches_the_cursor_parent_so_a_sibling_root_is_seen_when_it_appears() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("home");
    fs::create_dir_all(root.join(".cursor/projects")).unwrap();
    let events = Events::default();
    let sink = {
        let events = events.clone();
        Box::new(move |event: TailEvent| events.0.lock().unwrap().push(event))
            as Box<dyn Fn(TailEvent) + Send>
    };
    let tailer = Tailer::start(
        Store::open(temp.path().join("index.sqlite")).unwrap(),
        WatchConfig {
            home: root.clone(),
            hosts: vec![Host::Cursor],
            producer: ProducerSource::Checkout {
                pin: repo().join(".plugin-pin"),
                plugin_root: None,
            },
            python: None,
            debounce: Duration::from_millis(100),
            spawn_limits: xt_ingest::native::session_creation::spawn_limits(),
            probe: None,
        },
        sink,
    );
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(ready.freshness, Freshness::Live, "{ready:?}");
    let spelled = |path: &Path| path.to_string_lossy().into_owned();
    assert_eq!(
        tailer.status().watched,
        vec![
            // The home, without recursion, for the absent hook state pins.
            spelled(&root.canonicalize().unwrap()),
            spelled(&root.join(".cursor")),
            spelled(&root.join(".cursor/projects"))
        ]
    );
    let seen = tailer.status().reconciles;
    // The sibling root appears: the parent watch reports it, the host is
    // reconciled (the pinned producer is absent here, so explicitly not
    // imported), and the new root gains its own recursive watch.
    fs::create_dir_all(root.join(".cursor/chats/x")).unwrap();
    fs::write(root.join(".cursor/chats/x/store.db"), b"not a store").unwrap();
    settle(&tailer, seen);
    let watched = tailer.status().watched;
    assert!(
        watched.contains(&spelled(&root.join(".cursor/chats"))),
        "{watched:?}"
    );
    let last = events.reconciled();
    let TailEvent::Reconciled { report, .. } = last.last().unwrap() else {
        unreachable!()
    };
    assert_eq!(report.hosts[0].host, Host::Cursor);
    assert_eq!(
        report.hosts[0].status,
        HostStatus::PinMismatch,
        "{report:?}"
    );
    tailer.stop();
}

#[test]
fn claude_tail_watches_the_cursor_hook_state_pins_so_a_pin_change_is_reconciled() {
    // The hook's state pins fold into a Cursor session's clock and stamp and
    // change on their own, after the session's files stop changing.
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("home");
    fs::create_dir_all(root.join(".cursor/projects")).unwrap();
    let events = Events::default();
    let sink = {
        let events = events.clone();
        Box::new(move |event: TailEvent| events.0.lock().unwrap().push(event))
            as Box<dyn Fn(TailEvent) + Send>
    };
    let tailer = Tailer::start(
        Store::open(temp.path().join("index.sqlite")).unwrap(),
        WatchConfig {
            home: root.clone(),
            hosts: vec![Host::Cursor],
            producer: ProducerSource::Checkout {
                pin: repo().join(".plugin-pin"),
                plugin_root: None,
            },
            python: None,
            debounce: Duration::from_millis(100),
            spawn_limits: xt_ingest::native::session_creation::spawn_limits(),
            probe: None,
        },
        sink,
    );
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(ready.freshness, Freshness::Live, "{ready:?}");
    let spelled = |path: &Path| path.to_string_lossy().into_owned();
    let pins = root.join(".config/memhub-plugin/cursorflush");
    let pin = pins.join("00000000-0000-4000-8000-0000000000c6.json");
    // No `.config` yet: the home watch stands in for the pin directory. Its
    // appearance, with a first pin, is reported, the host is reconciled and
    // the directory gains its own recursive watch.
    let seen = tailer.status().reconciles;
    fs::create_dir_all(&pins).unwrap();
    fs::write(&pin, b"{}").unwrap();
    let seen = settle(&tailer, seen);
    let watched = tailer.status().watched;
    assert!(watched.contains(&spelled(&pins)), "{watched:?}");
    // A pin rewritten on its own, with nothing under `.cursor` touched, is
    // reconciled through that watch.
    fs::write(&pin, b"{\"usage\":1}").unwrap();
    settle(&tailer, seen);
    let last = events.reconciled();
    let TailEvent::Reconciled { report, .. } = last.last().unwrap() else {
        unreachable!()
    };
    assert_eq!(report.hosts[0].host, Host::Cursor);
    tailer.stop();
}

#[test]
fn cursor_ide_appearance_recreation_and_wal_changes_are_watched_without_alias_duplicates() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let events = Events::default();
    let sink = {
        let events = events.clone();
        Box::new(move |event: TailEvent| events.0.lock().unwrap().push(event))
            as Box<dyn Fn(TailEvent) + Send>
    };
    let tailer = Tailer::start(
        Store::open(temp.path().join("index.sqlite")).unwrap(),
        WatchConfig {
            home: home.clone(),
            hosts: vec![Host::Cursor],
            producer: ProducerSource::Checkout {
                pin: repo().join(".plugin-pin"),
                plugin_root: None,
            },
            python: None,
            debounce: Duration::from_millis(100),
            spawn_limits: xt_ingest::native::session_creation::spawn_limits(),
            probe: None,
        },
        sink,
    );
    tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(tailer.status().watched.len(), 1, "one physical fallback");
    let relative = "Library/Application Support/Cursor/User/globalStorage";
    let global = home.join(relative);
    let mut seen = tailer.status().reconciles;
    fs::create_dir_all(&global).unwrap();
    fs::write(global.join("state.vscdb"), b"synthetic watch-only fixture").unwrap();
    seen = settle(&tailer, seen);
    assert!(tailer.status().watched.iter().any(|path| Path::new(path).canonicalize().ok() == Some(global.canonicalize().unwrap())));
    for spelling in [&home, &home.canonicalize().unwrap()] {
        // Only WAL bytes change; no indexed records or transcript activity.
        fs::write(
            spelling.join(relative).join("state.vscdb-wal"),
            b"synthetic WAL change",
        )
        .unwrap();
        seen = settle(&tailer, seen);
        let installed = tailer.status().watch_installs;
        tailer.inject_removed(&spelling.join(relative));
        seen = settle(&tailer, seen);
        assert!(tailer.status().watch_installs > installed);
    }
    fs::remove_file(global.join("state.vscdb-wal")).unwrap();
    fs::remove_file(global.join("state.vscdb")).unwrap();
    fs::remove_dir(&global).unwrap();
    seen = settle(&tailer, seen);
    fs::create_dir_all(&global).unwrap();
    seen = settle(&tailer, seen);
    fs::write(global.join("state.vscdb-wal"), b"after recreation").unwrap();
    settle(&tailer, seen);
    let status = tailer.status();
    let physical: std::collections::BTreeSet<_> = status
        .watched
        .iter()
        .map(|path| Path::new(path).canonicalize().unwrap())
        .collect();
    assert_eq!(physical.len(), status.watched.len());
    assert!(events.reconciled().iter().all(|event| matches!(event, TailEvent::Reconciled { report, .. } if report.hosts.len() == 1 && report.hosts[0].host == Host::Cursor)));
    assert_eq!(
        Store::open(temp.path().join("index.sqlite"))
            .unwrap()
            .counts()
            .unwrap()
            .records,
        0
    );
    tailer.stop();
}

#[test]
fn claude_tail_reconciles_changes_received_before_a_stop_request() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..2)).unwrap();
    // The first live reconciliation is held for a moment so a further change
    // and the stop request both queue behind it.
    let holding = Arc::new(AtomicBool::new(false));
    let held = Arc::new(AtomicBool::new(false));
    let probe: xt_ingest::native::watch::Probe = {
        let (holding, held) = (Arc::clone(&holding), Arc::clone(&held));
        Arc::new(move |point: ProbePoint<'_>| {
            if matches!(point, ProbePoint::Reconciling(_))
                && holding.load(Ordering::SeqCst)
                && !held.swap(true, Ordering::SeqCst)
            {
                std::thread::sleep(Duration::from_millis(600));
            }
        })
    };
    let (tailer, events) = home.start(Some(probe));
    tailer.wait_ready(WAIT).expect("ready");
    holding.store(true, Ordering::SeqCst);
    home.append(A, &line(2, A));
    let started = std::time::Instant::now();
    while !held.load(Ordering::SeqCst) {
        assert!(
            started.elapsed() < WAIT,
            "the held reconciliation never started"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // Received while the worker is busy, then stopped: both must land.
    home.append(A, &line(3, A));
    std::thread::sleep(Duration::from_millis(150));
    tailer.stop();
    let store = home.store();
    assert_eq!(
        records(&store, A),
        4,
        "a change received before the stop is indexed"
    );
    assert_eq!(
        checkpoint(&store, &home.file(A)).unwrap().1,
        file_len(&home.file(A))
    );
    assert!(matches!(
        events.0.lock().unwrap().last(),
        Some(TailEvent::Stopped {
            freshness: Freshness::Live
        })
    ));
}

#[test]
fn claude_tail_watches_a_root_replaced_at_the_same_path_again() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..2)).unwrap();
    let (tailer, _events) = home.start(None);
    tailer.wait_ready(WAIT).expect("ready");
    let store = home.store();
    assert_eq!(records(&store, A), 2);
    let installs = tailer.status().watch_installs;
    let projects = home.root.join(".claude/projects");
    // The whole projects root is deleted and recreated at the same path with
    // a new session: the watch was tied to the old directory.
    let seen = tailer.status().reconciles;
    fs::remove_dir_all(&projects).unwrap();
    fs::create_dir_all(&home.project).unwrap();
    fs::write(home.file(B), body(B, 0..3)).unwrap();
    let seen = settle(&tailer, seen);
    assert_eq!(records(&store, B), 3);
    assert!(
        tailer.status().watch_installs > installs,
        "the replacement root was watched again: {:?}",
        tailer.status()
    );
    // A later file below the replacement is seen through the new watch.
    fs::write(home.file(C), body(C, 0..1)).unwrap();
    settle(&tailer, seen);
    assert_eq!(records(&store, C), 1);
    assert_eq!(records(&store, A), 2, "history is kept");
    tailer.stop();
}

#[test]
fn claude_tail_refreshes_a_checkpoint_after_proving_an_unchanged_file_by_its_whole_prefix() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..3)).unwrap();
    let path = home.file(A);
    let mut store = home.store();
    let request = ImportRequest {
        home: &home.root,
        hosts: &[Host::Claude],
        producer: &ProducerSource::Checkout {
            pin: repo().join(".plugin-pin"),
            plugin_root: None,
        },
        python: None,
        observed_at: 1,
        cancel: None,
    };
    assert!(scan_native(&mut store, &request, ScanMode::Resume).complete());
    let ctime_of = |generation: &Generation| {
        let Generation::File { ctime_ns, .. } = generation;
        *ctime_ns
    };
    let recorded = ctime_of(&checkpoint(&store, &path).unwrap().0);
    // An identical rewrite moves the change time without changing a byte.
    std::thread::sleep(Duration::from_millis(20));
    let bytes = fs::read(&path).unwrap();
    fs::write(&path, &bytes).unwrap();
    let current =
        xt_ingest::native::checkpoint::FileIdentity::of(&fs::metadata(&path).unwrap()).ctime_ns;
    assert_ne!(current, recorded);
    let report = scan_native(&mut store, &request, ScanMode::Resume);
    assert!(report.complete(), "{report:?}");
    assert_eq!(
        report.hosts[0].sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 0,
            records_enriched: 0
        }
    );
    let (generation, position) = checkpoint(&store, &path).unwrap();
    assert_eq!(
        ctime_of(&generation),
        current,
        "the checkpoint now carries the current identity"
    );
    assert_eq!(position, file_len(&path));
    assert_eq!(records(&store, A), 3);
    // Proven cheaply from here on: the checkpoint stays as it is.
    assert!(scan_native(&mut store, &request, ScanMode::Resume).complete());
    assert_eq!(checkpoint(&store, &path).unwrap().0, generation);
}

#[test]
fn claude_tail_registers_a_root_again_after_the_platform_reports_it_removed() {
    // A platform whose watch dies with the directory (inotify) may see the
    // root recreated with its inode reused, so identity alone cannot tell;
    // the removal event itself makes the root's watch lost and it is
    // registered again, whatever its identity says.
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..2)).unwrap();
    let (tailer, _events) = home.start(None);
    tailer.wait_ready(WAIT).expect("ready");
    let before = tailer.status();
    let projects = home.root.join(".claude/projects");
    tailer.inject_removed(&projects);
    let seen = settle(&tailer, before.reconciles);
    let after = tailer.status();
    assert!(
        after.watch_installs > before.watch_installs,
        "the root was registered again: {after:?}"
    );
    assert!(
        after
            .watched
            .contains(&projects.to_string_lossy().into_owned()),
        "{after:?}"
    );
    assert_eq!(after.freshness, Freshness::Live);
    // The re-registered watch reports later changes.
    fs::write(home.file(B), body(B, 0..3)).unwrap();
    let seen = settle(&tailer, seen);
    assert_eq!(records(&home.store(), B), 3);
    // A removal reported at the home itself names no host, yet takes every
    // root with it: the roots are registered again and their hosts
    // reconciled all the same.
    let before = tailer.status();
    tailer.inject_removed(&home.root);
    let seen = settle(&tailer, seen);
    let after = tailer.status();
    assert!(
        after.watch_installs > before.watch_installs,
        "the root was registered again after its ancestor was reported removed: {after:?}"
    );
    fs::write(home.file(C), body(C, 0..1)).unwrap();
    settle(&tailer, seen);
    assert_eq!(records(&home.store(), C), 1);
    tailer.stop();
}

#[test]
fn claude_tail_reconciles_an_event_delivered_before_a_stop_is_honored() {
    // A stop request and, right behind it, an event reach the queue while
    // the worker is about to drain (here: still in the initial scan's last
    // probe). A stop landing inside a drain ends the drain at once; the
    // event delivered before the stop is honored is still reconciled first.
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..2)).unwrap();
    let (scanned_tx, scanned_rx) = std::sync::mpsc::channel::<()>();
    let probe: xt_ingest::native::watch::Probe = Arc::new(move |point: ProbePoint<'_>| {
        if matches!(point, ProbePoint::InitialScanDone) {
            let _ = scanned_tx.send(());
            std::thread::sleep(Duration::from_millis(80));
        }
    });
    let (tailer, events) = home.start(Some(probe));
    scanned_rx.recv_timeout(WAIT).expect("initial scan done");
    tailer.request_stop();
    tailer.inject_removed(&home.root.join(".claude/projects"));
    tailer.stop();
    assert!(
        events.reconciled().iter().any(|event| matches!(
            event,
            TailEvent::Reconciled {
                trigger: xt_ingest::native::watch::Trigger::Startup,
                ..
            }
        )),
        "the event delivered before the stop was honored was reconciled: {:?}",
        events.0.lock().unwrap()
    );
}

#[test]
fn claude_tail_reports_a_stop_short_of_the_awaited_reconciliations_as_unmet() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..2)).unwrap();
    let (tailer, _events) = home.start(None);
    tailer.wait_ready(WAIT).expect("ready");
    let seen = tailer.status().reconciles;
    // The stop wakes the waiter, but a count never reached is not met.
    tailer.request_stop();
    assert!(!tailer.wait_reconciled(seen + 100, WAIT));
    // A count already reached stays met after the stop.
    assert!(tailer.wait_reconciled(seen, WAIT));
    tailer.stop();
}

#[test]
fn claude_tail_follows_an_accepted_symlinked_home_for_the_fallback_watch() {
    // The home itself may be an alias (unlike a source root): with no root
    // yet, the fallback watch goes on the directory the alias names.
    let temp = tempfile::TempDir::new().unwrap();
    let real = temp.path().join("real-home");
    fs::create_dir_all(&real).unwrap();
    let alias = temp.path().join("home");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let events = Events::default();
    let sink = {
        let events = events.clone();
        Box::new(move |event: TailEvent| events.0.lock().unwrap().push(event))
            as Box<dyn Fn(TailEvent) + Send>
    };
    let tailer = Tailer::start(
        Store::open(temp.path().join("index.sqlite")).unwrap(),
        WatchConfig {
            home: alias.clone(),
            hosts: vec![Host::Claude],
            producer: ProducerSource::Checkout {
                pin: repo().join(".plugin-pin"),
                plugin_root: None,
            },
            python: None,
            debounce: Duration::from_millis(100),
            spawn_limits: xt_ingest::native::session_creation::spawn_limits(),
            probe: None,
        },
        sink,
    );
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert_eq!(ready.freshness, Freshness::Live, "{ready:?}");
    let watched = tailer.status().watched;
    assert!(
        watched.contains(&real.canonicalize().unwrap().to_string_lossy().into_owned()),
        "{watched:?}"
    );
    // The first root appearing through the alias is seen and imported.
    let seen = tailer.status().reconciles;
    let project = alias.join(".claude/projects/-Users-fixture");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join(format!("{A}.jsonl")), body(A, 0..2)).unwrap();
    settle(&tailer, seen);
    assert_eq!(
        records(&Store::open(temp.path().join("index.sqlite")).unwrap(), A),
        2
    );
    tailer.stop();
}

#[test]
fn claude_tail_rebuilds_its_watches_after_a_watcher_error() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    fs::write(home.file(A), body(A, 0..2)).unwrap();
    let (tailer, events) = home.start(None);
    tailer.wait_ready(WAIT).expect("ready");
    let installs = tailer.status().watch_installs;
    let seen = tailer.status().reconciles;
    // The platform reports an error: whatever watch it lost is dropped, every
    // host is rescanned, the roots are watched again and freshness is live
    // once more; the error stays on record.
    tailer.inject_watcher_error("synthetic watch loss");
    let seen = settle(&tailer, seen);
    let status = tailer.status();
    assert!(status.watch_installs > installs, "{status:?}");
    assert_eq!(status.freshness, Freshness::Live, "{status:?}");
    assert!(
        status
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("synthetic watch loss")),
        "{status:?}"
    );
    assert!(
        events.reconciled().iter().any(|event| matches!(
            event,
            TailEvent::Reconciled {
                trigger: xt_ingest::native::watch::Trigger::Rescan,
                ..
            }
        )),
        "the error triggered a rescan"
    );
    // Changes below the rebuilt watch still arrive.
    home.append(A, &line(2, A));
    settle(&tailer, seen);
    assert_eq!(records(&home.store(), A), 3);
    tailer.stop();
}

/// A schema-4 index resumed unchanged Claude files behind checkpoints written
/// before the writer derived human classification. Migration 5 invalidates
/// those checkpoints once; the watcher's normal initial scan replays each file
/// through the writer's classifier, which only enriches unknown facts.
#[test]
fn claude_tail_v4_upgrade_replays_unchanged_history_once_to_enrich_human_classification() {
    const MARKER: &str = "synthetic-replay-marker-5c1e";
    // Tool input and output are never kept; only the person's message has a
    // short preview.
    const UNKEPT: &str = "synthetic-tool-marker-9d2a";
    let temp = tempfile::TempDir::new().unwrap();
    let home = Home::new(temp.path());
    let uuid = |index: usize| format!("7777a0a1-7777-4777-8777-{index:012}");
    let row = |index: usize, role: &str, content: serde_json::Value| {
        json!({
            "uuid": uuid(index),
            "type": role,
            "sessionId": A,
            "entrypoint": "cli",
            "cwd": "/repo/fixture",
            "timestamp": format!("2026-09-07T12:00:{index:02}Z"),
            "message": {"role": role, "content": content}
        })
        .to_string()
            + "\n"
    };
    // Every explicit user row except the first is excluded by content, so a
    // role-based inference would disagree with the classifier.
    let rows = [
        row(
            0,
            "user",
            json!([{"type":"text","text":format!("  {MARKER} prompt")}]),
        ),
        row(
            1,
            "assistant",
            json!([{"type":"text","text":"answer"},{"type":"tool_use","id":"t1","name":"Read","input":{"path":UNKEPT}}]),
        ),
        row(
            2,
            "user",
            json!([{"type":"tool_result","tool_use_id":"t1","content":UNKEPT}]),
        ),
        row(
            3,
            "user",
            json!([{"type":"text","text":"<command-name>/synthetic"}]),
        ),
        row(
            4,
            "user",
            json!([{"type":"text","text":"[Request interrupted by user]"}]),
        ),
    ];
    fs::write(home.file(A), rows.concat()).unwrap();
    let path = home.file(A);
    let request = ImportRequest {
        home: &home.root,
        hosts: &[Host::Claude],
        producer: &ProducerSource::Checkout {
            pin: repo().join(".plugin-pin"),
            plugin_root: None,
        },
        python: None,
        observed_at: 1,
        cancel: None,
    };
    let classified = |store: &Store| {
        store
            .records(A)
            .unwrap()
            .into_iter()
            .map(|r| (r.uuid, r.session_id, r.classification.is_human, r.text_len))
            .collect::<Vec<_>>()
    };
    let mut store = home.store();
    assert!(scan_native(&mut store, &request, ScanMode::Resume).complete());
    let expected = classified(&store);
    assert_eq!(
        expected.iter().map(|r| r.2).collect::<Vec<_>>(),
        [
            Some(true),
            Some(false),
            Some(false),
            Some(false),
            Some(false)
        ]
    );
    assert_eq!(expected[0].3, Some(MARKER.chars().count() as i64 + 9));

    // Legacy facts: unknown classification and length, sealed receipt coverage
    // over that poorer measurement, and a checkpoint proving the file at EOF.
    let sql = Connection::open(&home.db).unwrap();
    sql.execute("UPDATE records SET is_human=NULL,text_len=NULL", [])
        .unwrap();
    let revision = |row: &xt_store::StoredRecord| {
        let projection = xt_store::measurement::Projection::from_stored(row).unwrap();
        (
            projection.field_mask(),
            format!("{:x}", Sha256::digest(projection.canonical_bytes())),
        )
    };
    let legacy = store.records(A).unwrap();
    store
        .insert_capture_receipt(
            &xt_store::ingest::CaptureReceipt {
                receipt_id: "synthetic-receipt".into(),
                session_id: A.into(),
                surface: Some("cli".into()),
                received_at: 1,
            },
            &legacy
                .iter()
                .map(|row| {
                    let (mask, revision) = revision(row);
                    xt_store::ingest::RecordCoverage {
                        record_uuid: row.uuid.clone(),
                        metric_field_mask: mask,
                        measurement_revision: revision,
                        digest_schema_version: 1,
                    }
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    // Without the reset, the proven checkpoint skips the file: nothing enriches.
    let report = scan_native(&mut store, &request, ScanMode::Resume);
    assert_eq!(
        report.hosts[0].sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 0,
            records_enriched: 0
        }
    );
    assert!(
        classified(&store)
            .iter()
            .all(|r| r.2.is_none() && r.3.is_none())
    );
    let legacy_checkpoint = checkpoint(&store, &path).unwrap();
    assert_eq!(legacy_checkpoint.1, file_len(&path));
    drop(store);
    sql.execute_batch(
        "DELETE FROM schema_version WHERE version>=5;
         ALTER TABLE pull_requests DROP COLUMN refresh_error; ALTER TABLE pull_requests DROP COLUMN last_attempted_at;
         ALTER TABLE tool_uses DROP COLUMN group_key;
         ALTER TABLE tool_uses DROP COLUMN group_version;
         ALTER TABLE tool_uses DROP COLUMN group_conflict;
         DROP TABLE confirmed_automated_inputs;
         DROP TABLE guardian_turn_inputs;
         DROP TABLE injected_context_inputs; DROP TABLE IF EXISTS task_notification_inputs; DROP TABLE IF EXISTS record_previews; DROP TABLE IF EXISTS human_input_adjustments; DROP TABLE IF EXISTS human_session_origins;
         DROP TABLE session_creation_relations;
         DROP TABLE session_creation_bootstrap; DROP TABLE cli_artifact_launch_owners; DROP TABLE claude_launch_groups; DROP TABLE claude_launch_group_members; DROP TABLE claude_launch_candidates; DROP TABLE claude_launch_staged_candidates;
         DROP INDEX sessions_host_native; DROP INDEX source_cursors_tail;",
    )
        .unwrap();
    let evidence = || {
        sql.prepare(
            "SELECT r.receipt_id,r.session_id,r.surface,r.received_at,r.coverage_sealed,
                    c.record_uuid,c.metric_field_mask,c.measurement_revision,c.digest_schema_version
             FROM capture_receipts r JOIN capture_record_coverage c USING(receipt_id)
             ORDER BY c.record_uuid",
        )
        .unwrap()
        .query_map([], |r| {
            (0..9)
                .map(|i| r.get::<_, rusqlite::types::Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    };
    let receipts = evidence();
    assert_eq!(receipts.len(), 5);
    let sources = home.hashes();

    // The app path: the tailer opens (and migrates) the store, then scans.
    let (tailer, _events) = home.start(None);
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert!(ready.report.complete(), "{ready:?}");
    assert_eq!(
        ready.report.hosts[0].sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 0,
            records_enriched: 5
        },
        "the unchanged file was replayed from byte zero"
    );
    tailer.stop();
    let store = home.store();
    assert_eq!(store.schema_version().unwrap(), 17);
    assert_eq!(classified(&store), expected);
    assert_metadata_only(&store, A);
    let rows = store.records(A).unwrap();
    assert_eq!(rows[1].tool_uses.len(), 1);
    assert!(rows[1].tool_uses[0].input_json.is_none());
    assert_eq!(evidence(), receipts, "receipt coverage is immutable");
    for (row, old) in rows.iter().zip(&legacy) {
        assert_ne!(
            revision(row),
            revision(old),
            "enrichment does not upgrade the receipt's measurement"
        );
    }
    let replayed = checkpoint(&store, &path).unwrap();
    assert_eq!(replayed.1, file_len(&path));
    assert_eq!(home.hashes(), sources, "sources are never modified");
    drop(store);

    // Restart: version 5 does not reset again, so the file is proven unchanged.
    let (tailer, _events) = home.start(None);
    let ready = tailer.wait_ready(WAIT).expect("ready");
    assert!(ready.report.complete(), "{ready:?}");
    assert_eq!(
        ready.report.hosts[0].sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 0,
            records_enriched: 0
        }
    );
    tailer.stop();
    let store = home.store();
    assert_eq!(checkpoint(&store, &path).unwrap(), replayed);
    assert_eq!(classified(&store), expected);
    assert_eq!(evidence(), receipts);
    // The person's message is the one input with a preview, filled by the
    // replay the upgrade asked for.
    let previews: Vec<(String, String)> = sql
        .prepare("SELECT record_uuid,text FROM record_previews")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(previews, [(uuid(0), format!("{MARKER} prompt"))]);
    drop(store);
    drop(sql);
    // No other transcript content reached any index file, including WAL
    // sidecars.
    for entry in fs::read_dir(temp.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let bytes = fs::read(&path).unwrap();
            assert!(
                !bytes.windows(UNKEPT.len()).any(|w| w == UNKEPT.as_bytes()),
                "{path:?} retained transcript content"
            );
        }
    }
}
