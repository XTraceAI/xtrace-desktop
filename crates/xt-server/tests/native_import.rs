//! The headless `import-native` command: a JSON report on stdout, exit 0 only
//! when every requested host imported completely, 2 for an explicit gap.
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

fn core() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xtrace-core"))
}

#[test]
fn import_native_reports_and_exits_by_coverage() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-synthetic");
    fs::create_dir_all(&project).unwrap();
    let session = "00000000-0000-4000-8000-00000000c1a0";
    fs::write(
        project.join(format!("{session}.jsonl")),
        format!(
            "{{\"uuid\":\"33333333-3333-4333-8333-000000000001\",\"type\":\"user\",\"sessionId\":\"{session}\",\"entrypoint\":\"cli\",\"timestamp\":\"2026-09-07T12:00:00Z\",\"message\":{{\"role\":\"user\",\"content\":\"Synthetic request\"}}}}\n\
             {{\"uuid\":\"33333333-3333-4333-8333-000000000002\",\"type\":\"assistant\",\"sessionId\":\"{session}\",\"timestamp\":\"2026-09-07T12:00:05Z\",\"message\":{{\"role\":\"assistant\",\"model\":\"synthetic-model\",\"content\":[{{\"type\":\"text\",\"text\":\"Synthetic answer\"}}],\"usage\":{{\"input_tokens\":4,\"output_tokens\":2}}}}}}\n"
        ),
    )
    .unwrap();
    let db = temp.path().join("index.sqlite");
    let output = core()
        .args(["import-native", "--db"])
        .arg(&db)
        .arg("--home")
        .arg(&home)
        .args(["--host", "claude"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["hosts"][0]["host"], "claude");
    assert_eq!(report["hosts"][0]["status"], "complete");
    assert_eq!(report["hosts"][0]["sessions"][0]["records_new"], 2);
    assert_eq!(
        report["hosts"][0]["sessions"][0]["native_session_id"],
        session
    );
    assert_eq!(report["hosts"][0]["sessions"][0]["source_surface"], "cli");
    assert!(
        report["hosts"][0]["sessions"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with(".jsonl")
    );

    // A relative --home is anchored to the invoking directory; reported paths
    // are absolute and the repeated import adds nothing.
    let output = core()
        .current_dir(temp.path())
        .args(["import-native", "--db"])
        .arg(&db)
        .args(["--home", "./home", "--host", "claude"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["hosts"][0]["status"], "complete");
    assert_eq!(report["hosts"][0]["sessions"][0]["records_new"], 0);
    let path = report["hosts"][0]["sessions"][0]["path"].as_str().unwrap();
    assert!(std::path::Path::new(path).is_absolute(), "{path}");
    assert!(!path.contains("/./"), "{path}");

    // A requested host without sources is an explicit gap, not a silent success.
    let output = core()
        .args(["import-native", "--db"])
        .arg(&db)
        .arg("--home")
        .arg(&home)
        .args(["--host", "codex"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["hosts"][0]["status"], "missing_source");
    assert!(
        report["hosts"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("codex")
    );

    // Invalid invocations fail before touching anything.
    for args in [
        vec!["import-native", "--home"],
        vec!["import-native", "--db", "x", "--host", "other"],
        vec![
            "import-native",
            "--db",
            "x",
            "--home",
            ".",
            "--host",
            "claude",
            "--host",
            "claude",
        ],
    ] {
        let output = core().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
    }
    let output = core()
        .args(["import-native", "--db"])
        .arg(&db)
        .arg("--home")
        .arg(temp.path().join("nope"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--home"));
}

#[cfg(unix)]
#[test]
fn database_and_sidecar_aliases_cannot_modify_native_history() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let sources = home.join(".cursor/chats/session");
    fs::create_dir_all(&sources).unwrap();
    let source = sources.join("store.db");
    fs::write(&source, "").unwrap();
    let link = temp.path().join("linked.sqlite");
    symlink(&source, &link).unwrap();
    let hard = temp.path().join("hard.sqlite");
    fs::hard_link(&source, &hard).unwrap();
    let sidecar_db = temp.path().join("sidecar.sqlite");
    symlink(&source, temp.path().join("sidecar.sqlite-wal")).unwrap();
    // The hook state area the Cursor watch covers is no place for the index
    // either: its own writes would read as changes.
    let pins = home.join(".config/memhub-plugin/cursorflush");
    fs::create_dir_all(&pins).unwrap();
    for db in [
        &source,
        &link,
        &hard,
        &sidecar_db,
        &sources.join("new.sqlite"),
        &pins.join("index.sqlite"),
        &home.join(".config/memhub-plugin/index.sqlite"),
    ] {
        let output = core()
            .args(["import-native", "--db"])
            .arg(db)
            .arg("--home")
            .arg(&home)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(fs::read(&source).unwrap(), b"");
    }
    assert!(!sidecar_db.exists());
    assert!(!sources.join("new.sqlite").exists());
    assert_eq!(fs::read_dir(&sources).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&pins).unwrap().count(), 0);
    assert!(!home.join(".config/memhub-plugin/index.sqlite").exists());
}

#[cfg(unix)]
#[test]
fn native_reverse_symlinks_cannot_expose_the_index_as_source_history() {
    use std::os::unix::fs::symlink;
    for directory in [false, true] {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("home");
        let source = home.join(".cursor/chats/session");
        fs::create_dir_all(&source).unwrap();
        let data = temp.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let db = data.join("store.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("CREATE TABLE native_data(value TEXT)", [])
            .unwrap();
        drop(conn);
        let original = fs::read(&db).unwrap();
        if directory {
            symlink(&data, source.join("linked")).unwrap();
        } else {
            symlink(&db, source.join("store.db")).unwrap();
        }
        let output = core()
            .args(["import-native", "--db"])
            .arg(&db)
            .arg("--home")
            .arg(&home)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(fs::read(&db).unwrap(), original);
        assert_eq!(fs::read_dir(&data).unwrap().count(), 1);
    }
}

/// `watch-native --once`: the watcher registers, the initial scan runs, queued
/// changes are reconciled, one `ready` line is printed, and the process exits
/// by freshness and completeness. A second run resumes behind checkpoints.
#[test]
fn watch_native_once_reports_ready_and_resumes_on_the_next_run() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-synthetic");
    fs::create_dir_all(&project).unwrap();
    let session = "00000000-0000-4000-8000-00000000c1b0";
    let record = |index: u32| {
        format!(
            "{{\"uuid\":\"44444444-4444-4444-8444-{index:012}\",\"type\":\"user\",\"sessionId\":\"{session}\",\"entrypoint\":\"cli\",\"timestamp\":\"2026-09-07T12:00:0{index}Z\",\"message\":{{\"role\":\"user\",\"content\":\"turn {index}\"}}}}\n"
        )
    };
    fs::write(
        project.join(format!("{session}.jsonl")),
        record(1) + &record(2),
    )
    .unwrap();
    let db = temp.path().join("index.sqlite");
    let run = || {
        core()
            .args(["watch-native", "--db"])
            .arg(&db)
            .arg("--home")
            .arg(&home)
            .args(["--host", "claude", "--once"])
            .output()
            .unwrap()
    };
    let output = run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    // A `startup` reconciliation may precede `ready` (the platform may report
    // a change made just before the watch); the readiness report counts both.
    let ready_of = |lines: &[Value]| {
        lines
            .iter()
            .find(|line| line["event"] == "ready")
            .cloned()
            .unwrap_or_else(|| panic!("no ready event: {lines:?}"))
    };
    let ready = ready_of(&lines);
    assert_eq!(ready["freshness"], "live");
    assert_eq!(ready["report"]["hosts"][0]["status"], "complete");
    assert_eq!(ready["report"]["hosts"][0]["sessions"][0]["records_new"], 2);
    assert_eq!(lines.last().unwrap()["event"], "stopped");
    assert_eq!(lines.last().unwrap()["freshness"], "live");
    // Appended offline; the next run reads only the new record.
    fs::OpenOptions::new()
        .append(true)
        .open(project.join(format!("{session}.jsonl")))
        .unwrap()
        .write_all(record(3).as_bytes())
        .unwrap();
    let output = run();
    assert!(output.status.success());
    let lines: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let ready = ready_of(&lines);
    assert_eq!(ready["report"]["hosts"][0]["sessions"][0]["records_new"], 1);
    assert_eq!(
        ready["report"]["hosts"][0]["sessions"][0]["source_surface"],
        "cli"
    );
    // `--replay` on the one-shot import rereads everything and adds nothing.
    let output = core()
        .args(["import-native", "--db"])
        .arg(&db)
        .arg("--home")
        .arg(&home)
        .args(["--host", "claude", "--replay"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["hosts"][0]["sessions"][0]["records_new"], 0);
    assert_eq!(report["hosts"][0]["sessions"][0]["source_surface"], "cli");
    // Usage errors stay explicit.
    for args in [
        vec![
            "watch-native",
            "--db",
            "x",
            "--home",
            ".",
            "--once",
            "--once",
        ],
        vec!["watch-native", "--db", "x", "--home", ".", "--for", "soon"],
        vec!["import-native", "--db", "x", "--home", ".", "--once"],
    ] {
        let output = core().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
    }
}

/// A synthetic Claude home with one two-record transcript; returns the home,
/// the transcript path and the index path.
fn synthetic_home(
    temp: &std::path::Path,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let home = temp.join("home");
    let project = home.join(".claude/projects/-Users-synthetic");
    fs::create_dir_all(&project).unwrap();
    let transcript = project.join(format!("{WATCH_SESSION}.jsonl"));
    fs::write(&transcript, watch_record(1) + &watch_record(2)).unwrap();
    (home, transcript, temp.join("index.sqlite"))
}

const WATCH_SESSION: &str = "00000000-0000-4000-8000-00000000c1b1";

fn watch_record(index: u32) -> String {
    format!(
        "{{\"uuid\":\"45454545-4545-4545-8545-{index:012}\",\"type\":\"user\",\"sessionId\":\"{WATCH_SESSION}\",\"entrypoint\":\"cli\",\"timestamp\":\"2026-09-07T12:00:0{index}Z\",\"message\":{{\"role\":\"user\",\"content\":\"turn {index}\"}}}}\n"
    )
}

/// Wait for the child to exit, killing it and failing if it has not within
/// `limit`, so a hung watcher fails the test instead of hanging it.
fn exit_within(child: &mut Child, limit: Duration) -> ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the watcher did not exit within {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn watch_native_stops_and_fails_when_its_event_stream_cannot_be_written() {
    let temp = tempfile::TempDir::new().unwrap();
    let (home, transcript, db) = synthetic_home(temp.path());
    let mut child = core()
        .args(["watch-native", "--db"])
        .arg(&db)
        .arg("--home")
        .arg(&home)
        .args(["--host", "claude", "--for", "120"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let ready: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(ready["event"], "ready");
    // The consumer goes away (as with `| head -1`); the next event cannot be
    // written, which stops the watcher long before `--for` elapses and
    // fails the run, instead of running on with no observable output.
    drop(reader);
    fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap()
        .write_all(watch_record(3).as_bytes())
        .unwrap();
    let status = exit_within(&mut child, Duration::from_secs(60));
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert_eq!(status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("Could not write the event stream"),
        "{stderr}"
    );
    // The change that could not be reported was still indexed before the stop.
    assert_eq!(
        xt_store::Store::open(&db)
            .unwrap()
            .records(WATCH_SESSION)
            .unwrap()
            .len(),
        3
    );
}

#[cfg(unix)]
#[test]
fn watch_native_stops_cleanly_on_ctrl_c_with_a_single_runtime_worker() {
    let temp = tempfile::TempDir::new().unwrap();
    let (home, _transcript, db) = synthetic_home(temp.path());
    // One runtime worker, as on a single-core host: the main thread waits
    // while the signal listener runs on that worker, so the interrupt still
    // reaches the tailer and it stops after its final reconciliation.
    let mut child = core()
        .env("TOKIO_WORKER_THREADS", "1")
        .args(["watch-native", "--db"])
        .arg(&db)
        .arg("--home")
        .arg(&home)
        .args(["--host", "claude"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&first).unwrap()["event"],
        "ready"
    );
    assert!(
        Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let status = exit_within(&mut child, Duration::from_secs(60));
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert_eq!(
        status.code(),
        Some(0),
        "the interrupt stopped the tailer instead of killing the process: {stderr}"
    );
    let mut rest = String::new();
    reader.read_to_string(&mut rest).unwrap();
    let last: Value = serde_json::from_str(rest.lines().last().unwrap()).unwrap();
    assert_eq!(last["event"], "stopped");
    assert_eq!(last["freshness"], "live");
}

#[test]
fn watch_native_reports_a_bound_reached_during_startup_as_not_ready() {
    let temp = tempfile::TempDir::new().unwrap();
    let (home, _transcript, db) = synthetic_home(temp.path());
    // A bound of zero seconds is reached before the initial scan can finish:
    // whatever the scan emits, the run did not become ready within its bound.
    let output = core()
        .args(["watch-native", "--db"])
        .arg(&db)
        .arg("--home")
        .arg(&home)
        .args(["--host", "claude", "--for", "0"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("before it was ready"), "{stderr}");
    // The scan itself completed and its rows stay indexed.
    assert_eq!(
        xt_store::Store::open(&db)
            .unwrap()
            .records(WATCH_SESSION)
            .unwrap()
            .len(),
        2
    );
}
