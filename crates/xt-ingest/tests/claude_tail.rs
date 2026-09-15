//! Incremental scanning and live tailing of native Claude history: the
//! watcher is registered before the initial scan, changes made during the scan
//! reach the index before ready with no later event, live appends, completed
//! partial lines, new files and coalesced directory events converge within
//! seconds, truncation, replacement, restart and a failed transaction never
//! omit or duplicate a record, sources are never modified, storage stays
//! metadata-only, and zero-position locators from the initial importer migrate
//! to checkpoints by one full replay.
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
    HostStatus, ImportRequest, ScanMode, SessionOutcome,
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
                pin: repo().join(".plugin-pin"),
                plugin_root: None,
                python: None,
                debounce: Duration::from_millis(100),
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

const WAIT: Duration = Duration::from_secs(10);

/// Wait for a reconciliation newer than `after`, then for the worker to stay
/// idle with no newer reconciliation for a quiet period, so a burst the
/// platform delivered in pieces has fully settled. Returns the count seen.
fn settle(tailer: &Tailer, after: u64) -> u64 {
    assert!(
        tailer.wait_reconciled(after + 1, WAIT),
        "no reconciliation within {WAIT:?}: {:?}",
        tailer.status()
    );
    for _ in 0..40 {
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
    let inode = |generation: &Generation| match generation {
        Generation::File { ino, .. } => *ino,
        Generation::HostScan { .. } => unreachable!(),
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
        "only the appended record is read after a restart"
    );
    assert_eq!(
        outcome(B),
        SessionOutcome::Imported {
            records_new: 0,
            records_enriched: 0
        }
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
        pin: &repo().join(".plugin-pin"),
        plugin_root: None,
        python: None,
        observed_at: 1,
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
    // Proven unchanged: the next scan touches nothing.
    let report = scan_native(&mut store, &request, ScanMode::Resume);
    assert!(report.complete());
    assert_eq!(records(&store, A), 3);
    // A replay rereads everything and still adds nothing.
    let report = scan_native(&mut store, &request, ScanMode::Replay);
    assert!(report.complete());
    assert_eq!(records(&store, A), 3);
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
            pin: repo().join(".plugin-pin"),
            plugin_root: None,
            python: None,
            debounce: Duration::from_millis(100),
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
        vec![home.root.to_string_lossy().into_owned()]
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
            pin: repo().join(".plugin-pin"),
            plugin_root: None,
            python: None,
            debounce: Duration::from_millis(100),
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
        pin: &repo().join(".plugin-pin"),
        plugin_root: None,
        python: None,
        observed_at: 1,
    };
    assert!(scan_native(&mut store, &request, ScanMode::Resume).complete());
    let ctime_of = |generation: &Generation| match generation {
        Generation::File { ctime_ns, .. } => *ctime_ns,
        Generation::HostScan { .. } => unreachable!(),
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
