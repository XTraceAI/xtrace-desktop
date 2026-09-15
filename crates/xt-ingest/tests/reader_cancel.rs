//! Cancelling native scans: a reader that never returns is killed and reaped,
//! the sessions it completed stay committed and the host reads cancelled; a
//! Claude scan ends between files; a cancelled tailer stops within its bound
//! rather than waiting for the reader; and a scan cancelled before it started
//! reads nothing.
//!
//! The hung reader is a fake interpreter: a shell script that answers the
//! version probe, streams one complete session and the header of a second,
//! then sleeps for good. The real pinned bundle is what it is asked to run,
//! so the production path is exercised end to end.
#![cfg(unix)]
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use xt_ingest::native::{
    CancelToken, HostStatus, ImportRequest, ProducerSource, ScanMode, SessionOutcome,
    import_native,
    readers_cli::{ReaderError, read_pin, resolve_python_within},
    scan_native, scan_native_observed,
    watch::{TailEvent, Tailer, WatchConfig},
};
use xt_store::{Host, Store};

const WAIT: Duration = Duration::from_secs(20);

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bundle() -> ProducerSource {
    ProducerSource::Bundle {
        pin: read_pin(&repo().join(".plugin-pin")).unwrap(),
        root: repo().join("vendor/agent-plugins"),
    }
}

fn header(native: &str) -> String {
    json!({
        "type": "session", "host": "codex", "native_session_id": native,
        "conversation_id": format!("codex-{native}"), "source_surface": "codex_cli",
        "started_at": "2026-09-07T12:00:00.000Z", "cwd": "/repo/fixture", "git_branch": null,
        "title": null, "path": format!("$HOME/.codex/sessions/{native}.jsonl"), "mtime": 1788782400.0
    })
    .to_string()
}

fn record(native: &str, index: usize) -> String {
    let role = if index.is_multiple_of(2) {
        "user"
    } else {
        "assistant"
    };
    json!({
        "uuid": format!("7777{}-7777-4777-8777-{index:012}", &native[native.len() - 4..]),
        "type": role, "cwd": "/repo/fixture",
        "timestamp": format!("2026-09-07T12:00:{index:02}Z"),
        "message": {"role": role, "content": [{"type": "text", "text": format!("turn {index}")}]}
    })
    .to_string()
}

const FIRST: &str = "00000000-0000-4000-8000-00000000aaaa";
const SECOND: &str = "00000000-0000-4000-8000-00000000bbbb";

/// A fake interpreter that passes the runtime probe, then, run as the reader,
/// records its process ID under the home, streams the first session whole and
/// the second session's header, and never returns.
fn hung_python(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = root.join("python3");
    let lines = [
        header(FIRST),
        record(FIRST, 0),
        record(FIRST, 1),
        header(SECOND),
    ]
    .iter()
    .map(|line| format!("printf '%s\\n' '{line}'\n"))
    .collect::<String>();
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"-c\" ]; then echo True; exit 0; fi\n\
             echo $$ > \"$HOME/reader.pid\"\n{lines}exec sleep 1000\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn codex_home(root: &Path) -> PathBuf {
    let home = root.join("home");
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    home
}

/// The process ID the fake reader recorded, once it runs.
fn reader_pid(home: &Path) -> i32 {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(text) = fs::read_to_string(home.join("reader.pid"))
            && let Ok(pid) = text.trim().parse()
        {
            return pid;
        }
        assert!(Instant::now() < deadline, "the reader never started");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn alive(pid: i32) -> bool {
    // A reaped process has no ID to signal; a killed but unreaped one still
    // has, so this is false only once the child was waited for.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[test]
fn a_cancelled_reader_is_killed_and_reaped_and_its_completed_sessions_stay() {
    let temp = tempfile::TempDir::new().unwrap();
    let python = hung_python(temp.path());
    let home = codex_home(temp.path());
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let producer = bundle();
    let cancel = CancelToken::new();
    let (report, pid) = std::thread::scope(|scope| {
        let scan = scope.spawn(|| {
            import_native(
                &mut store,
                &ImportRequest {
                    home: &home,
                    hosts: &[Host::Codex, Host::Cursor],
                    producer: &producer,
                    python: Some(python.as_os_str()),
                    observed_at: 1,
                    cancel: Some(&cancel),
                },
            )
        });
        let pid = reader_pid(&home);
        assert!(alive(pid));
        let started = Instant::now();
        cancel.cancel();
        let report = scan.join().unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the cancelled scan returned only after {:?}",
            started.elapsed()
        );
        (report, pid)
    });
    assert!(!alive(pid), "the killed reader was not reaped");
    let codex = &report.hosts[0];
    assert_eq!(codex.status, HostStatus::Cancelled, "{report:?}");
    assert!(codex.detail.as_deref().unwrap().contains("cancelled"));
    // The session the reader completed before the kill is indexed; the one it
    // had opened is reported as ended early, without a cursor.
    assert_eq!(codex.sessions.len(), 2, "{codex:?}");
    assert!(matches!(
        codex.sessions[0].outcome,
        SessionOutcome::Imported { records_new: 2, .. }
    ));
    assert!(matches!(
        &codex.sessions[1].outcome,
        SessionOutcome::Skipped { reason } if reason.contains("ended before completing")
    ));
    assert_eq!(store.counts().unwrap().records, 2);
    // The host after the cancelled one was not started at all.
    assert_eq!(report.hosts[1].status, HostStatus::Cancelled);
    assert!(
        report.hosts[1]
            .detail
            .as_deref()
            .unwrap()
            .contains("before this host was read")
    );
    assert!(!report.complete());
}

#[test]
fn a_tailer_shut_down_during_a_hung_initial_scan_stops_within_its_bound() {
    let temp = tempfile::TempDir::new().unwrap();
    let python = hung_python(temp.path());
    let home = codex_home(temp.path());
    let store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let events = Arc::clone(&events);
        Box::new(move |event: TailEvent| events.lock().unwrap().push(event))
    };
    let tailer = Tailer::start(
        store,
        WatchConfig {
            home: home.clone(),
            hosts: vec![Host::Codex],
            producer: bundle(),
            python: Some(python.into_os_string()),
            debounce: Duration::from_millis(100),
            probe: None,
        },
        sink,
    );
    let pid = reader_pid(&home);
    assert!(tailer.wait_ready(Duration::from_millis(500)).is_none());
    let started = Instant::now();
    assert!(
        tailer.shutdown(WAIT),
        "the worker did not end within the bound"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "shutdown took {:?}",
        started.elapsed()
    );
    assert!(!alive(pid), "the killed reader was not reaped");
    let events = events.lock().unwrap();
    assert!(
        matches!(events.as_slice(), [TailEvent::Stopped { .. }]),
        "a scan cancelled before ready publishes no readiness: {events:?}"
    );
}

#[test]
fn cancel_then_a_graceful_stop_does_not_wait_for_the_hung_reader() {
    let temp = tempfile::TempDir::new().unwrap();
    let python = hung_python(temp.path());
    let home = codex_home(temp.path());
    let store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let tailer = Tailer::start(
        store,
        WatchConfig {
            home: home.clone(),
            hosts: vec![Host::Codex],
            producer: bundle(),
            python: Some(python.into_os_string()),
            debounce: Duration::from_millis(100),
            probe: None,
        },
        Box::new(|_| {}),
    );
    let pid = reader_pid(&home);
    let started = Instant::now();
    tailer.cancel();
    tailer.stop();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(!alive(pid));
}

fn claude_home(root: &Path, files: usize) -> PathBuf {
    let home = root.join("home");
    let project = home.join(".claude/projects/-repo-fixture");
    fs::create_dir_all(&project).unwrap();
    for index in 0..files {
        let session = format!("00000000-0000-4000-8000-0000000{index:05}");
        let line = json!({
            "uuid": format!("8888{}-8888-4888-8888-000000000000", &session[session.len() - 4..]),
            "type": "user", "sessionId": session, "entrypoint": "cli", "cwd": "/repo/fixture",
            "timestamp": "2026-09-07T12:00:00Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "turn"}]}
        });
        fs::write(
            project.join(format!("{session}.jsonl")),
            format!("{line}\n"),
        )
        .unwrap();
    }
    home
}

#[test]
fn a_scan_cancelled_before_it_started_reads_nothing_and_says_so() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path(), 2);
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    let producer = bundle();
    let report = scan_native(
        &mut store,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Claude, Host::Codex],
            producer: &producer,
            python: None,
            observed_at: 1,
            cancel: Some(&cancel),
        },
        ScanMode::Resume,
    );
    for host in &report.hosts {
        assert_eq!(host.status, HostStatus::Cancelled, "{host:?}");
        assert!(host.sessions.is_empty());
    }
    assert_eq!(store.counts().unwrap().sessions, 0);
}

#[test]
fn a_claude_scan_cancelled_between_files_keeps_what_it_read() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path(), 3);
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let cancel = CancelToken::new();
    let producer = bundle();
    let report = scan_native_observed(
        &mut store,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Claude],
            producer: &producer,
            python: None,
            observed_at: 1,
            cancel: Some(&cancel),
        },
        ScanMode::Resume,
        &mut |_| cancel.cancel(),
    );
    let claude = &report.hosts[0];
    assert_eq!(claude.status, HostStatus::Cancelled, "{report:?}");
    assert_eq!(claude.sessions.len(), 1);
    assert!(matches!(
        claude.sessions[0].outcome,
        SessionOutcome::Imported { records_new: 1, .. }
    ));
    assert!(
        claude
            .detail
            .as_deref()
            .unwrap()
            .contains("before every transcript")
    );
    assert_eq!(store.counts().unwrap().sessions, 1);
    // The next, uncancelled scan reads the rest and proves the first file's
    // checkpoint: nothing is duplicated.
    let report = scan_native(
        &mut store,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Claude],
            producer: &producer,
            python: None,
            observed_at: 2,
            cancel: None,
        },
        ScanMode::Resume,
    );
    assert_eq!(report.hosts[0].status, HostStatus::Complete);
    assert_eq!(store.counts().unwrap().sessions, 3);
    assert_eq!(store.counts().unwrap().records, 3);
}

/// A fake interpreter that never answers the version probe; it records its
/// process ID at `pid_file` (baked in: probes inherit the real environment).
fn hung_probe_python(root: &Path, pid_file: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = root.join("python3-hung-probe");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\necho $$ > \"{}\"\nexec sleep 1000\n",
            pid_file.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn pid_at(path: &Path) -> i32 {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(text) = fs::read_to_string(path)
            && let Ok(pid) = text.trim().parse()
        {
            return pid;
        }
        assert!(Instant::now() < deadline, "the process never started");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_interpreter_probe_that_never_answers_is_bounded_and_cancellable() {
    let temp = tempfile::TempDir::new().unwrap();
    let pid_file = temp.path().join("probe.pid");
    let python = hung_probe_python(temp.path(), &pid_file);
    // Bounded: a probe still running at the deadline is killed and reported.
    // The probe runs on its own thread so the fake's start (a shell under
    // load can take a while) is awaited independently of the deadline.
    let result = std::thread::scope(|scope| {
        let probe = scope.spawn(|| {
            let started = Instant::now();
            let result =
                resolve_python_within(Some(python.as_os_str()), None, Duration::from_secs(3));
            (result, started.elapsed())
        });
        let pid = pid_at(&pid_file);
        let (result, elapsed) = probe.join().unwrap();
        assert!(!alive(pid), "the timed-out probe was not reaped");
        assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
        result
    });
    assert!(
        matches!(&result, Err(ReaderError::MissingRuntime(reason)) if reason.contains("did not finish within 3s")),
        "{result:?}"
    );
    fs::remove_file(&pid_file).unwrap();
    // Cancellable: a cancel during the probe kills it at once.
    let cancel = CancelToken::new();
    let result = std::thread::scope(|scope| {
        let probe = scope.spawn(|| {
            resolve_python_within(
                Some(python.as_os_str()),
                Some(&cancel),
                Duration::from_secs(60),
            )
        });
        let pid = pid_at(&pid_file);
        let started = Instant::now();
        cancel.cancel();
        let result = probe.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!alive(pid), "the cancelled probe was not reaped");
        result
    });
    assert!(matches!(result, Err(ReaderError::Cancelled)), "{result:?}");
    fs::remove_file(&pid_file).unwrap();
    // A cancelled token refuses the next probe without starting it.
    assert!(matches!(
        resolve_python_within(
            Some(python.as_os_str()),
            Some(&cancel),
            Duration::from_secs(60)
        ),
        Err(ReaderError::Cancelled)
    ));
    assert!(!pid_file.exists());
}
