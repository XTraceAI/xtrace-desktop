//! The app's native index lifecycle without a Tauri runtime: startup against
//! a synthetic home and the app's database, typed status through readiness
//! and later appends, explicit reporting of what the reader hosts lack, and a
//! shutdown that does not wait for a reader that never returns.
#![cfg(unix)]
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use xt_store::Store;
use xtrace_desktop::dto::{
    NativeFreshness, NativeHostState, NativeIndexPhase, NativeIndexStatus, PythonRuntime,
    ReaderBundle,
};
use xtrace_desktop::native_index::{NativeIndex, NativeIndexOptions, PIN, Publish};

const WAIT: Duration = Duration::from_secs(30);

fn bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../vendor/agent-plugins")
}

fn transcript_line(session: &str, index: usize) -> String {
    let role = if index.is_multiple_of(2) {
        "user"
    } else {
        "assistant"
    };
    json!({
        "uuid": format!("9999{}-9999-4999-8999-{index:012}", &session[session.len() - 4..]),
        "type": role, "sessionId": session, "entrypoint": "cli", "cwd": "/repo/fixture",
        "timestamp": format!("2026-09-07T12:00:{index:02}Z"),
        "message": {"role": role, "content": [{"type": "text", "text": format!("turn {index}")}]}
    })
    .to_string()
}

const SESSION: &str = "00000000-0000-4000-8000-00000000c1a0";

fn synthetic_home(root: &Path) -> PathBuf {
    let home = root.join("home");
    let project = home.join(".claude/projects/-repo-fixture");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join(format!("{SESSION}.jsonl")),
        format!(
            "{}\n{}\n",
            transcript_line(SESSION, 0),
            transcript_line(SESSION, 1)
        ),
    )
    .unwrap();
    home
}

fn recorder() -> (Publish, Arc<Mutex<Vec<NativeIndexStatus>>>) {
    let published = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&published);
    (
        Arc::new(move |status: &NativeIndexStatus| sink.lock().unwrap().push(status.clone())),
        published,
    )
}

fn wait_for(index: &NativeIndex, done: impl Fn(&NativeIndexStatus) -> bool) -> NativeIndexStatus {
    let deadline = Instant::now() + WAIT;
    loop {
        let status = index.status();
        if done(&status) {
            return status;
        }
        assert!(Instant::now() < deadline, "timed out waiting on {status:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn host<'a>(
    status: &'a NativeIndexStatus,
    name: &str,
) -> &'a xtrace_desktop::dto::NativeHostStatus {
    status.hosts.iter().find(|host| host.host == name).unwrap()
}

#[test]
fn indexes_a_synthetic_home_publishes_typed_status_and_reconciles_appends() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = synthetic_home(temp.path());
    let data = temp.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let db = data.join("xtrace.db");
    let (publish, published) = recorder();
    let index = NativeIndex::start(
        NativeIndexOptions {
            home: home.clone(),
            db: db.clone(),
            bundle: bundle(),
            python: None,
        },
        publish,
    );
    let first = published.lock().unwrap().first().cloned().unwrap();
    assert_eq!(first.phase, NativeIndexPhase::Scanning);
    assert!(
        first
            .hosts
            .iter()
            .all(|h| h.state == NativeHostState::Pending)
    );
    let pin: serde_json::Value = serde_json::from_str(PIN).unwrap();
    assert_eq!(
        first.readers,
        ReaderBundle::Verified {
            commit: pin["commit"].as_str().unwrap().to_owned(),
            plugin_version: pin["plugin_version"].as_str().unwrap().to_owned(),
        }
    );
    // Discovery for the status runs beside the initial scan; both finish.
    let ready = wait_for(&index, |s| {
        s.phase == NativeIndexPhase::Ready && s.python != PythonRuntime::Resolving
    });
    assert_eq!(ready.freshness, NativeFreshness::Live);
    let claude = host(&ready, "claude");
    assert_eq!(claude.state, NativeHostState::Complete, "{ready:?}");
    assert_eq!((claude.sessions_imported, claude.records_new), (1, 2));
    for name in ["codex", "cursor"] {
        // No Codex/Cursor history under this home: the source is what is
        // missing, whatever interpreter the machine has (sources are checked
        // before the runtime).
        let reader = host(&ready, name);
        assert_eq!(reader.state, NativeHostState::MissingSource, "{reader:?}");
    }
    assert!(
        matches!(
            ready.python,
            PythonRuntime::Available { .. } | PythonRuntime::Missing { .. }
        ),
        "{:?}",
        ready.python
    );
    let store = Store::open(&db).unwrap();
    assert_eq!(store.counts().unwrap().records, 2);
    // An append is reconciled live, through the checkpoint: one new record.
    let reconciles = ready.reconciles;
    let path = home.join(format!(".claude/projects/-repo-fixture/{SESSION}.jsonl"));
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&transcript_line(SESSION, 2));
    text.push('\n');
    fs::write(&path, text).unwrap();
    let after = wait_for(&index, |s| {
        s.reconciles > reconciles && host(s, "claude").records_new == 1
    });
    assert_eq!(host(&after, "claude").state, NativeHostState::Complete);
    assert_eq!(store.counts().unwrap().records, 3);
    // Shutdown is prompt and the final status says stopped; a restart resumes
    // behind the checkpoint and adds nothing.
    let started = Instant::now();
    assert!(index.shutdown());
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(index.status().phase, NativeIndexPhase::Stopped);
    assert!(published.lock().unwrap().last().unwrap().phase == NativeIndexPhase::Stopped);
    assert!(index.shutdown(), "a second shutdown is a no-op");
    let (publish, _) = recorder();
    let again = NativeIndex::start(
        NativeIndexOptions {
            home,
            db: db.clone(),
            bundle: bundle(),
            python: None,
        },
        publish,
    );
    let ready = wait_for(&again, |s| s.phase == NativeIndexPhase::Ready);
    let claude = host(&ready, "claude");
    assert_eq!(
        (claude.sessions_imported, claude.records_new),
        (1, 0),
        "{ready:?}"
    );
    assert_eq!(store.counts().unwrap().records, 3);
    assert!(again.shutdown());
}

#[test]
fn a_data_directory_inside_the_native_sources_disables_the_index_with_the_reason() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = synthetic_home(temp.path());
    let (publish, published) = recorder();
    let index = NativeIndex::start(
        NativeIndexOptions {
            home: home.clone(),
            db: home.join(".claude/xtrace.db"),
            bundle: bundle(),
            python: None,
        },
        publish,
    );
    let status = index.status();
    assert!(
        matches!(&status.phase, NativeIndexPhase::Disabled { reason } if reason.contains("native history")),
        "{status:?}"
    );
    assert_eq!(published.lock().unwrap().len(), 1);
    assert!(index.shutdown());
    assert!(!home.join(".claude/xtrace.db").exists());
}

#[test]
fn a_missing_interpreter_and_a_missing_bundle_are_reported_while_claude_indexes() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = synthetic_home(temp.path());
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    let (publish, _) = recorder();
    let index = NativeIndex::start(
        NativeIndexOptions {
            home,
            db: temp.path().join("xtrace.db"),
            bundle: temp.path().join("no-bundle"),
            python: Some("/nonexistent/python3".into()),
        },
        publish,
    );
    let ready = wait_for(&index, |s| {
        s.phase == NativeIndexPhase::Ready && s.python != PythonRuntime::Resolving
    });
    assert!(
        matches!(&ready.python, PythonRuntime::Missing { reason } if reason.contains("python"))
    );
    assert!(
        matches!(&ready.readers, ReaderBundle::Unavailable { reason } if reason.contains("bundled"))
    );
    assert_eq!(host(&ready, "claude").state, NativeHostState::Complete);
    let codex = host(&ready, "codex");
    // The runtime is resolved before the producer: a Codex history with no
    // interpreter reports the runtime, explicitly.
    assert_eq!(codex.state, NativeHostState::MissingRuntime, "{codex:?}");
    assert!(codex.detail.as_deref().unwrap().contains("python"));
    assert!(index.shutdown());
}

/// A fake interpreter that passes the runtime probe and, as the reader,
/// records its process ID under the home and never returns.
fn hung_python(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = root.join("python3");
    fs::write(
        &script,
        "#!/bin/sh\nif [ \"$1\" = \"-c\" ]; then echo True; exit 0; fi\n\
         echo $$ > \"$HOME/reader.pid\"\nexec sleep 1000\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[test]
fn a_hung_reader_does_not_hold_shutdown_and_is_reaped() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = synthetic_home(temp.path());
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    let python = hung_python(temp.path());
    let (publish, _) = recorder();
    let index = NativeIndex::start(
        NativeIndexOptions {
            home: home.clone(),
            db: temp.path().join("xtrace.db"),
            bundle: bundle(),
            python: Some(python.into_os_string()),
        },
        publish,
    );
    let deadline = Instant::now() + WAIT;
    let pid: i32 = loop {
        if let Ok(text) = fs::read_to_string(home.join("reader.pid"))
            && let Ok(pid) = text.trim().parse()
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "the reader never started");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(index.status().phase, NativeIndexPhase::Scanning);
    let started = Instant::now();
    assert!(index.shutdown(), "the worker did not end within the bound");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "shutdown took {:?}",
        started.elapsed()
    );
    assert_eq!(index.status().phase, NativeIndexPhase::Stopped);
    // Killed and reaped: no process by that ID remains.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
}

/// A fake interpreter that never answers the version probe; it records its
/// process ID under `root` (the path is baked in: probes inherit the real
/// environment, not the indexed home's).
fn hung_probe_python(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = root.join("python3-hung-probe");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\necho $$ > \"{}\"\nexec sleep 1000\n",
            root.join("probe.pid").display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[test]
fn an_interpreter_that_never_answers_its_probe_blocks_neither_startup_nor_shutdown() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = synthetic_home(temp.path());
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    let python = hung_probe_python(temp.path());
    let (publish, _) = recorder();
    let started = Instant::now();
    let index = NativeIndex::start(
        NativeIndexOptions {
            home: home.clone(),
            db: temp.path().join("xtrace.db"),
            bundle: bundle(),
            python: Some(python.into_os_string()),
        },
        publish,
    );
    // Startup does not wait for the probe; the status says it is resolving.
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(index.status().python, PythonRuntime::Resolving);
    let deadline = Instant::now() + WAIT;
    let pid: i32 = loop {
        if let Ok(text) = fs::read_to_string(temp.path().join("probe.pid"))
            && let Ok(pid) = text.trim().parse()
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "the probe never started");
        std::thread::sleep(Duration::from_millis(20));
    };
    // Claude is indexed meanwhile: the Codex scan waits on its own probe,
    // which the shutdown below kills as well.
    let ready = wait_for(&index, |s| {
        host(s, "claude").state == NativeHostState::Complete
    });
    assert_eq!(host(&ready, "claude").records_new, 2);
    let started = Instant::now();
    assert!(index.shutdown());
    assert!(started.elapsed() < Duration::from_secs(3));
    // Every probe registered with the token was killed and reaped.
    let gone = Instant::now() + Duration::from_secs(5);
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(Instant::now() < gone, "the hung probe survived shutdown");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_unreadable_project_directory_is_reported_by_the_index_status() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::TempDir::new().unwrap();
    let home = synthetic_home(temp.path());
    let sealed = home.join(".claude/projects/-repo-sealed");
    fs::create_dir_all(&sealed).unwrap();
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000)).unwrap();
    let (publish, _) = recorder();
    let index = NativeIndex::start(
        NativeIndexOptions {
            home: home.clone(),
            db: temp.path().join("xtrace.db"),
            bundle: bundle(),
            python: None,
        },
        publish,
    );
    let ready = wait_for(&index, |s| s.phase == NativeIndexPhase::Ready);
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o755)).unwrap();
    // The readable transcript is indexed; the sealed directory is a diagnostic
    // that keeps the host incomplete, visible in the status.
    let claude = host(&ready, "claude");
    assert_eq!(claude.state, NativeHostState::Incomplete, "{ready:?}");
    assert_eq!((claude.sessions_imported, claude.records_new), (1, 2));
    assert!(claude.diagnostics >= 1, "{claude:?}");
    assert!(index.shutdown());
}

#[test]
fn the_compiled_pin_parses_and_the_status_serializes_with_tagged_variants() {
    use xt_ingest::native::readers_cli::parse_pin;
    assert!(parse_pin(PIN).is_ok());
    let (publish, _) = recorder();
    let disabled = NativeIndex::disabled("fixture mode uses a disposable database", publish);
    let value = serde_json::to_value(disabled.status()).unwrap();
    assert_eq!(value["phase"]["phase"], "disabled");
    assert_eq!(value["freshness"]["freshness"], "unknown");
    assert_eq!(value["python"]["state"], "missing");
    assert_eq!(value["readers"]["state"], "unavailable");
    assert_eq!(value["hosts"], json!([]));
}

/// Runs in a child process of this test binary (see the test below), where
/// the environment can carry a stale `PYTHON` without racing other tests.
#[test]
fn discovery_child_ignores_the_inherited_python_variable() {
    if std::env::var_os("XTRACE_TEST_DISCOVERY_CHILD").is_none() {
        return;
    }
    use xt_ingest::native::readers_cli::{ReaderError, discover_python};
    let stale = std::env::var("PYTHON").unwrap();
    match discover_python(None, None) {
        Ok(path) => assert_ne!(path.to_string_lossy(), stale),
        Err(ReaderError::MissingRuntime(reason)) => {
            assert!(!reason.contains(&stale), "{reason}");
            assert!(reason.contains("python3 on PATH"), "{reason}");
        }
        Err(other) => panic!("{other:?}"),
    }
}

#[test]
fn an_inherited_python_variable_does_not_narrow_desktop_discovery() {
    // The desktop honors only XTRACE_PYTHON; a `PYTHON` a shell exported is
    // not a choice, so discovery still tries PATH and the known directories.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "discovery_child_ignores_the_inherited_python_variable",
            "--nocapture",
        ])
        .env("XTRACE_TEST_DISCOVERY_CHILD", "1")
        .env("PYTHON", "/nonexistent/xtrace-python3")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
