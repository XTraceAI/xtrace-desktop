//! Reading one Codex or Cursor session through the pinned producer's
//! exact-detail mode.
//!
//! The synthetic cases run a fake interpreter — a shell script standing in for
//! Python — against the real verified bundle, so the supervision is exercised
//! exactly as production runs it: the arguments the reader is given, the
//! identity its output must carry, the bounds on what it may write and how
//! long it may run, and that a run which is not a complete validated success
//! returns nothing at all and leaves no process behind.
//!
//! `conformance_exact_detail` runs the real pinned producer over F18's
//! synthetic native files and a paginated Codex group built from local synthetic
//! data in bundle mode or the pinned checkout's own history fixture in checkout
//! mode: the JSONL sessions load whole and match the
//! ordinary export record for record, the SQLite store is refused — with a
//! committed write-ahead log beside it — and nothing under the home or in the
//! test-owned reader temporary directory changes.
#![cfg(unix)]
use serde_json::json;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use xt_ingest::native::{
    CancelToken, ProducerSource,
    readers_cli::{
        DetailBounds, DetailFailure, DetailLimit, DetailRefusal, PinnedProducer, read_pin,
        resolve_python, spawn_reader,
    },
    session_source::{
        DetailReader, SessionSourceOutcome, SessionSourceRequest, SourceCeiling, SourceLimits,
        SourceUnavailable, load_session_source, load_session_source_with,
    },
    stream::{StreamEvent, StreamEvents},
};
use xt_store::{Host, SessionSource};

const CODEX: &str = "00000000-0000-4000-8000-00000000c0de";
#[path = "support/conformance_source.rs"]
mod conformance_source;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The real bundled producer, verified against the pin. The fake interpreter
/// is handed its script and ignores it.
fn producer() -> PinnedProducer {
    ProducerSource::Bundle {
        pin: read_pin(&repo().join(".plugin-pin")).unwrap(),
        root: repo().join("vendor/agent-plugins"),
    }
    .producer()
    .expect("the bundle verifies against the pin")
}

struct Scene {
    _temp: tempfile::TempDir,
    /// Outside the home: where a fake interpreter leaves its evidence.
    root: PathBuf,
    home: PathBuf,
}

fn scene() -> Scene {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let home = root.join("home");
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    Scene {
        _temp: temp,
        root,
        home,
    }
}

/// A fake interpreter whose body is `script`. `$ROOT` is the scene's evidence
/// directory; the reader's own environment is cleared, so it is baked in.
fn fake(scene: &Scene, script: &str) -> OsString {
    let path = scene.root.join("python3");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nROOT='{}'\n{script}\n",
            scene.root.to_str().unwrap()
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path.into_os_string()
}

fn header_for(native: &str, home: &Path) -> serde_json::Value {
    json!({
        "type": "session", "host": "codex", "native_session_id": native,
        "conversation_id": format!("codex-{native}"), "source_surface": "codex_cli",
        "started_at": "2026-09-07T12:00:00.000Z", "cwd": "/repo/fixture", "git_branch": null,
        "title": null,
        "path": home.join(format!(".codex/sessions/rollout-{native}.jsonl")).to_str().unwrap(),
        "mtime": 1788782400.0
    })
}

fn record(index: usize) -> serde_json::Value {
    let role = if index.is_multiple_of(2) {
        "user"
    } else {
        "assistant"
    };
    json!({
        "uuid": format!("77770000-7777-4777-8777-{index:012}"),
        "type": role, "cwd": "/repo/fixture",
        "timestamp": format!("2026-09-07T12:00:{index:02}Z"),
        "message": {"role": role, "content": [{"type": "text", "text": format!("turn {index}")}]}
    })
}

/// Shell lines printing each value as one line of stdout.
fn prints(lines: &[serde_json::Value]) -> String {
    lines
        .iter()
        .map(|line| format!("printf '%s\\n' '{line}'\n"))
        .collect()
}

/// A whole, valid session for `CODEX` under the scene's home.
fn session(scene: &Scene, records: usize) -> String {
    let mut lines = vec![header_for(CODEX, &scene.home)];
    lines.extend((0..records).map(record));
    prints(&lines)
}

fn bounds(deadline_ms: u64, grace_ms: u64) -> DetailBounds {
    DetailBounds {
        deadline: Duration::from_millis(deadline_ms),
        grace: Duration::from_millis(grace_ms),
        ..DetailBounds::default()
    }
}

fn load(
    scene: &Scene,
    python: &OsString,
    host: Host,
    native: &str,
    bounds: DetailBounds,
    cancel: Option<&CancelToken>,
    limits: SourceLimits,
) -> SessionSourceOutcome {
    let producer = producer();
    load_session_source_with(
        &SessionSourceRequest {
            home: &scene.home,
            host,
            native_session_id: native,
            indexed: &[],
            cancel,
            limits,
        },
        Some(&DetailReader {
            python,
            producer: &producer,
            bounds,
        }),
    )
}

fn load_codex(scene: &Scene, python: &OsString) -> SessionSourceOutcome {
    load(
        scene,
        python,
        Host::Codex,
        CODEX,
        DetailBounds::default(),
        None,
        SourceLimits::default(),
    )
}

fn refused(outcome: SessionSourceOutcome) -> SourceUnavailable {
    match outcome {
        SessionSourceOutcome::Unavailable(reason) => reason,
        SessionSourceOutcome::Loaded(loaded) => {
            panic!("returned {} records", loaded.records.len())
        }
    }
}

fn protocol(outcome: SessionSourceOutcome) -> &'static str {
    match refused(outcome) {
        SourceUnavailable::Reader(DetailFailure::Protocol(reason)) => reason,
        other => panic!("{other:?}"),
    }
}

/// Every file under `dir`, with its bytes and modification time.
fn tree(dir: &Path) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                // A configured root may be an alias: its target, not its
                // contents, is what belongs to this entry.
                let target = fs::read_link(&path).unwrap();
                out.insert(
                    path.clone(),
                    (
                        target.into_os_string().into_encoded_bytes(),
                        meta.modified().unwrap(),
                    ),
                );
            } else if meta.is_dir() {
                walk(&path, out);
            } else {
                out.insert(
                    path.clone(),
                    (fs::read(&path).unwrap(), meta.modified().unwrap()),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, &mut out);
    out
}

/// Whether the process is gone: killed and reaped by someone. An orphan is
/// reaped by init shortly after its group is killed, so this waits for that.
fn gone(pid: i32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        // SAFETY: signal 0 only asks whether the process exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn pid_in(path: &Path) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(text) = fs::read_to_string(path)
            && let Ok(pid) = text.trim().parse()
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_reader_is_invoked_in_exact_detail_mode_for_exactly_the_stored_identity() {
    let scene = scene();
    let python = fake(
        &scene,
        &format!(
            "printf '%s\\n' \"$@\" > \"$ROOT/argv\"\n\
             {{ pwd; printf '%s\\n' \"$HOME\" \"$CODEX_HOME\" \"$PYTHONDONTWRITEBYTECODE\"; }} > \"$ROOT/env\"\n{}",
            session(&scene, 2)
        ),
    );
    let loaded = match load_codex(&scene, &python) {
        SessionSourceOutcome::Loaded(loaded) => loaded,
        other => panic!("{other:?}"),
    };
    assert_eq!(loaded.records.len(), 2);
    assert_eq!(loaded.native_session_id, CODEX);
    assert_eq!(loaded.host, Host::Codex);
    assert!(loaded.gaps.is_empty());
    assert_eq!(
        loaded.sources[0].path,
        scene
            .home
            .join(format!(".codex/sessions/rollout-{CODEX}.jsonl"))
    );
    let argv = fs::read_to_string(scene.root.join("argv")).unwrap();
    let argv: Vec<&str> = argv.lines().collect();
    assert_eq!(
        argv[1..],
        [
            "--host",
            "codex",
            "--exact-detail",
            &format!("--session={CODEX}"),
            "--deadline-seconds",
            "30.000"
        ]
    );
    assert!(argv[0].ends_with("plugins/memhub/scripts/readers_cli.py"));
    let env = fs::read_to_string(scene.root.join("env")).unwrap();
    let home = scene.home.to_str().unwrap();
    assert_eq!(
        env.lines().collect::<Vec<_>>(),
        [home, home, &format!("{home}/.codex"), "1"]
    );
}

#[test]
fn an_identity_that_could_name_a_path_or_a_ranking_starts_no_process() {
    let scene = scene();
    let python = fake(&scene, "touch \"$ROOT/started\"");
    for native in [
        "", "latest", "../x", "a/b", "a\\b", ".hidden", " padded", "tab\t",
    ] {
        for host in [Host::Codex, Host::Cursor] {
            assert_eq!(
                refused(load(
                    &scene,
                    &python,
                    host,
                    native,
                    DetailBounds::default(),
                    None,
                    SourceLimits::default()
                )),
                SourceUnavailable::InvalidIdentifier,
                "{native:?}"
            );
        }
    }
    // Without a reader, the hosts are refused as before, before anything runs.
    for host in [Host::Codex, Host::Cursor] {
        assert_eq!(
            refused(load_session_source(&SessionSourceRequest {
                home: &scene.home,
                host,
                native_session_id: CODEX,
                indexed: &[],
                cancel: None,
                limits: SourceLimits::default(),
            })),
            SourceUnavailable::PrerequisiteUnavailable
        );
    }
    assert!(!scene.root.join("started").exists());
}

#[test]
fn only_the_requested_session_under_this_home_is_accepted() {
    let scene = scene();
    let other = "00000000-0000-4000-8000-00000000beef";
    let outside = json!({
        "path": "/elsewhere/.codex/sessions/rollout.jsonl",
    });
    let mut moved = header_for(CODEX, &scene.home);
    moved["path"] = outside["path"].clone();
    let mut relative = header_for(CODEX, &scene.home);
    relative["path"] = json!("home/.codex/sessions/rollout.jsonl");
    // Inside the home, but not under the host's root: the home is not the
    // boundary.
    let mut beside = header_for(CODEX, &scene.home);
    beside["path"] = json!(scene.home.join(".codex/rollout.jsonl").to_str().unwrap());
    let mut climbing = header_for(CODEX, &scene.home);
    climbing["path"] = json!(
        scene
            .home
            .join(".codex/sessions/../../secret.jsonl")
            .to_str()
            .unwrap()
    );
    let mut foreign_record = record(1);
    foreign_record["sessionId"] = json!(other);
    let mut cursor_header = header_for(CODEX, &scene.home);
    cursor_header["host"] = json!("cursor");
    cursor_header["conversation_id"] = json!(format!("cursor-{CODEX}"));
    for (case, lines, expected) in [
        (
            "another session",
            vec![header_for(other, &scene.home), record(0)],
            "session header names another session",
        ),
        (
            "a source outside the home",
            vec![moved, record(0)],
            "session source is outside this host's roots",
        ),
        (
            "a source beside the root",
            vec![beside, record(0)],
            "session source is outside this host's roots",
        ),
        (
            "a source climbing out of the root",
            vec![climbing, record(0)],
            "session source is outside this host's roots",
        ),
        (
            "a relative source",
            vec![relative, record(0)],
            "session source is outside this host's roots",
        ),
        (
            "a second session",
            vec![
                header_for(CODEX, &scene.home),
                record(0),
                header_for(CODEX, &scene.home),
            ],
            "reader wrote more than one session",
        ),
        (
            "a record of another session",
            vec![header_for(CODEX, &scene.home), record(0), foreign_record],
            "a record names another session",
        ),
        (
            "another host",
            vec![cursor_header, record(0)],
            "reader output does not match the shared stream contract",
        ),
        (
            "a record before any header",
            vec![record(0), header_for(CODEX, &scene.home)],
            "a record precedes the session header",
        ),
        ("nothing at all", vec![], "reader wrote no session"),
    ] {
        let python = fake(&scene, &prints(&lines));
        let reason = match refused(load_codex(&scene, &python)) {
            SourceUnavailable::Reader(DetailFailure::Protocol(reason)) => reason,
            // An empty stream does not end in a newline either.
            other => panic!("{case}: {other:?}"),
        };
        if case == "nothing at all" {
            assert_eq!(reason, "reader output ends inside a line", "{case}");
        } else {
            assert_eq!(reason, expected, "{case}");
        }
    }
}

#[test]
fn a_truncated_or_malformed_stream_returns_nothing() {
    let scene = scene();
    let whole = session(&scene, 3);
    // The last line printed without its newline: cut short, exit 0 or not.
    let cut = format!(
        "{}printf '%s' '{}'\n",
        prints(&[header_for(CODEX, &scene.home), record(0)]),
        record(1)
    );
    assert_eq!(
        protocol(load_codex(&scene, &fake(&scene, &cut))),
        "reader output ends inside a line"
    );
    let garbage = format!("{whole}printf '%s\\n' 'not json at all'\n");
    assert_eq!(
        protocol(load_codex(&scene, &fake(&scene, &garbage))),
        "reader output does not match the shared stream contract"
    );
    let bad_utf8 = format!("{whole}printf '\\377\\n'\n");
    assert_eq!(
        protocol(load_codex(&scene, &fake(&scene, &bad_utf8))),
        "reader output is not UTF-8"
    );
    // A diagnostic beside a complete session is not a complete success.
    let noisy = format!(
        "{whole}printf '%s\\n' '{}' >&2\n",
        json!({"type": "diagnostic", "host": "codex", "code": "source_changed", "path": null})
    );
    assert_eq!(
        protocol(load_codex(&scene, &fake(&scene, &noisy))),
        "reader wrote diagnostics beside a complete session"
    );
}

#[test]
fn a_nonzero_exit_after_a_whole_session_discards_it() {
    let scene = scene();
    let whole = session(&scene, 4);
    let diagnostic = |code: &str| {
        format!(
            "printf '%s\\n' '{}' >&2\n",
            json!({"type": "diagnostic", "host": "codex", "code": code, "path": "/secret/path"})
        )
    };
    for (exit, stderr, expected) in [
        (
            1,
            String::new(),
            DetailFailure::Protocol("reader exited abnormally"),
        ),
        (
            2,
            String::new(),
            DetailFailure::Protocol("reader refused without a known diagnostic"),
        ),
        (
            2,
            diagnostic("detail_limit_records"),
            DetailFailure::Refused(DetailRefusal::Limit(DetailLimit::Records)),
        ),
        (
            2,
            diagnostic("source_changed"),
            DetailFailure::Refused(DetailRefusal::SourceChanged),
        ),
        (
            2,
            diagnostic("not_a_code_we_know"),
            DetailFailure::Protocol("reader refused without a known diagnostic"),
        ),
        (
            2,
            "echo 'Traceback: /secret/path' >&2\n".to_owned(),
            DetailFailure::Protocol("reader refused without a known diagnostic"),
        ),
        (
            137,
            String::new(),
            DetailFailure::Protocol("reader exited abnormally"),
        ),
    ] {
        let python = fake(&scene, &format!("{whole}{stderr}exit {exit}"));
        assert_eq!(
            refused(load_codex(&scene, &python)),
            SourceUnavailable::Reader(expected),
            "exit {exit} {stderr}"
        );
    }
}

#[test]
fn every_declared_refusal_is_its_own_closed_reason() {
    let scene = scene();
    for (code, expected) in [
        (
            "detail_limit_source_bytes",
            DetailRefusal::Limit(DetailLimit::SourceBytes),
        ),
        (
            "detail_limit_files",
            DetailRefusal::Limit(DetailLimit::Files),
        ),
        (
            "detail_limit_native_rows",
            DetailRefusal::Limit(DetailLimit::NativeRows),
        ),
        (
            "detail_limit_records",
            DetailRefusal::Limit(DetailLimit::Records),
        ),
        (
            "detail_limit_line_bytes",
            DetailRefusal::Limit(DetailLimit::LineBytes),
        ),
        (
            "detail_limit_output_bytes",
            DetailRefusal::Limit(DetailLimit::OutputBytes),
        ),
        (
            "detail_limit_header_bytes",
            DetailRefusal::Limit(DetailLimit::HeaderBytes),
        ),
        (
            "detail_limit_discovery_entries",
            DetailRefusal::Limit(DetailLimit::DiscoveryEntries),
        ),
        (
            "detail_limit_header_probe_bytes",
            DetailRefusal::Limit(DetailLimit::HeaderProbeBytes),
        ),
        (
            "detail_limit_probe_bytes",
            DetailRefusal::Limit(DetailLimit::ProbeBytes),
        ),
        (
            "detail_limit_probes",
            DetailRefusal::Limit(DetailLimit::Probes),
        ),
        ("detail_deadline", DetailRefusal::Deadline),
        (
            "detail_prerequisite_unsupported",
            DetailRefusal::StoreUnsupported,
        ),
        ("source_changed", DetailRefusal::SourceChanged),
        ("session_unavailable", DetailRefusal::Unavailable),
        ("session_unreadable", DetailRefusal::Unreadable),
        ("discovery_incomplete", DetailRefusal::DiscoveryIncomplete),
    ] {
        let python = fake(
            &scene,
            &format!(
                "printf '%s\\n' '{}' >&2\nexit 2",
                json!({"type": "diagnostic", "host": "codex", "code": code, "path": null})
            ),
        );
        assert_eq!(
            refused(load_codex(&scene, &python)),
            SourceUnavailable::Reader(DetailFailure::Refused(expected)),
            "{code}"
        );
    }
    // A diagnostic naming another host is not this reader's.
    let python = fake(
        &scene,
        &format!(
            "printf '%s\\n' '{}' >&2\nexit 2",
            json!({"type": "diagnostic", "host": "cursor", "code": "detail_deadline", "path": null})
        ),
    );
    assert_eq!(
        protocol(load_codex(&scene, &python)),
        "reader refused without a known diagnostic"
    );
}

#[test]
fn output_past_its_bound_kills_the_reader_and_keeps_nothing() {
    let scene = scene();
    let small = DetailBounds {
        max_stdout: 64 * 1024,
        max_stderr: 4 * 1024,
        ..bounds(20_000, 1_000)
    };
    for (stream, flood) in [("stdout", "yes '{}'"), ("stderr", "yes x >&2")] {
        let python = fake(
            &scene,
            &format!(
                "echo $$ > \"$ROOT/{stream}.pid\"\n{}{flood}",
                session(&scene, 1)
            ),
        );
        let started = Instant::now();
        assert_eq!(
            refused(load(
                &scene,
                &python,
                Host::Codex,
                CODEX,
                small,
                None,
                SourceLimits::default()
            )),
            SourceUnavailable::Reader(DetailFailure::OutputBound),
            "{stream}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{stream} was not stopped at its bound"
        );
        assert!(gone(pid_in(&scene.root.join(format!("{stream}.pid")))));
    }
    // The record ceiling of the request is its own, counted in records.
    let python = fake(&scene, &session(&scene, 5));
    assert_eq!(
        refused(load(
            &scene,
            &python,
            Host::Codex,
            CODEX,
            DetailBounds::default(),
            None,
            SourceLimits {
                max_records: 3,
                ..SourceLimits::default()
            }
        )),
        SourceUnavailable::TooLarge {
            limit: SourceCeiling::Records,
            reached: 4,
            ceiling: 3
        }
    );
}

#[test]
fn a_reader_past_the_hard_deadline_is_killed_with_its_group() {
    let scene = scene();
    // It writes a whole session, then neither exits nor honours the deadline
    // it was handed; something it started holds on as well.
    let python = fake(
        &scene,
        &format!(
            "echo $$ > \"$ROOT/leader.pid\"\n(sleep 1000 >/dev/null 2>&1) &\n\
             echo $! > \"$ROOT/child.pid\"\n{}exec sleep 1000",
            session(&scene, 2)
        ),
    );
    let started = Instant::now();
    assert_eq!(
        refused(load(
            &scene,
            &python,
            Host::Codex,
            CODEX,
            bounds(2_000, 500),
            None,
            SourceLimits::default()
        )),
        SourceUnavailable::Reader(DetailFailure::Deadline)
    );
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(2_500), "{took:?}");
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert!(gone(pid_in(&scene.root.join("leader.pid"))));
    assert!(gone(pid_in(&scene.root.join("child.pid"))));
}

#[test]
fn a_leader_that_exits_while_its_output_is_held_open_is_not_a_success() {
    let scene = scene();
    // A whole session and exit 0, but a process it started still holds the
    // output: the stream is not known to be complete until it closes.
    let python = fake(
        &scene,
        &format!(
            "{}sleep 1000 &\necho $! > \"$ROOT/holder.pid\"\nexit 0",
            session(&scene, 2)
        ),
    );
    let started = Instant::now();
    assert_eq!(
        refused(load(
            &scene,
            &python,
            Host::Codex,
            CODEX,
            bounds(2_000, 500),
            None,
            SourceLimits::default()
        )),
        SourceUnavailable::Reader(DetailFailure::Deadline)
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(gone(pid_in(&scene.root.join("holder.pid"))));
}

/// A root that is a symlink to elsewhere is the reader's supported layout,
/// and it reports the source by where the root led; that path is accepted,
/// and a path under the root's target that the home does not lead to is not.
#[test]
fn a_symlinked_root_is_anchored_where_it_led_before_the_read() {
    let scene = scene();
    let target = scene.root.join("elsewhere/codex-sessions");
    fs::create_dir_all(&target).unwrap();
    fs::remove_dir(scene.home.join(".codex/sessions")).unwrap();
    std::os::unix::fs::symlink(&target, scene.home.join(".codex/sessions")).unwrap();
    let mut header = header_for(CODEX, &scene.home);
    header["path"] = json!(target.join("rollout.jsonl").to_str().unwrap());
    let python = fake(&scene, &prints(&[header, record(0)]));
    let SessionSourceOutcome::Loaded(loaded) = load_codex(&scene, &python) else {
        panic!("a source under the root's target is the session's");
    };
    assert_eq!(loaded.sources[0].path, target.join("rollout.jsonl"));
    let mut stray = header_for(CODEX, &scene.home);
    stray["path"] = json!(scene.root.join("elsewhere/rollout.jsonl").to_str().unwrap());
    let python = fake(&scene, &prints(&[stray, record(0)]));
    assert_eq!(
        protocol(load_codex(&scene, &python)),
        "session source is outside this host's roots"
    );
}

/// The process state `ps` reports, e.g. `Z` for a zombie.
fn state_of(pid: i32) -> String {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The race a registered child loses: the leader has already exited and is a
/// zombie awaiting its reaper, while a process it started still holds the
/// output open. A cancel landing now must still kill that process: only the
/// supervisor reaps, so the group is signalled while its ID is still the
/// leader's. Deterministic — the cancel is sent only once `ps` shows the
/// leader as a zombie.
#[test]
fn a_cancel_after_the_leader_exited_still_kills_what_holds_its_output() {
    let scene = scene();
    let python = fake(
        &scene,
        &format!(
            "echo $$ > \"$ROOT/leader.pid\"\n{}sleep 1000 &\n\
             echo $! > \"$ROOT/holder.pid\"\nexit 0",
            session(&scene, 2)
        ),
    );
    let cancel = CancelToken::new();
    let canceller = {
        let cancel = cancel.clone();
        let leader = scene.root.join("leader.pid");
        let holder = scene.root.join("holder.pid");
        std::thread::spawn(move || {
            let leader = pid_in(&leader);
            pid_in(&holder);
            let deadline = Instant::now() + Duration::from_secs(10);
            while !state_of(leader).starts_with('Z') {
                assert!(Instant::now() < deadline, "the leader never exited");
                std::thread::sleep(Duration::from_millis(10));
            }
            cancel.cancel();
        })
    };
    let started = Instant::now();
    assert_eq!(
        refused(load(
            &scene,
            &python,
            Host::Codex,
            CODEX,
            bounds(30_000, 5_000),
            Some(&cancel),
            SourceLimits::default()
        )),
        SourceUnavailable::Cancelled
    );
    canceller.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(gone(pid_in(&scene.root.join("holder.pid"))));
    assert!(gone(pid_in(&scene.root.join("leader.pid"))));
}

#[test]
fn a_cancel_kills_the_group_and_returns_nothing() {
    let scene = scene();
    let python = fake(
        &scene,
        &format!(
            "echo $$ > \"$ROOT/leader.pid\"\n(sleep 1000 >/dev/null 2>&1) &\n\
             echo $! > \"$ROOT/child.pid\"\n{}exec sleep 1000",
            session(&scene, 2)
        ),
    );
    let cancel = CancelToken::new();
    let canceller = {
        let cancel = cancel.clone();
        let leader = scene.root.join("leader.pid");
        let child = scene.root.join("child.pid");
        std::thread::spawn(move || {
            pid_in(&leader);
            pid_in(&child);
            std::thread::sleep(Duration::from_millis(100));
            cancel.cancel();
        })
    };
    let started = Instant::now();
    assert_eq!(
        refused(load(
            &scene,
            &python,
            Host::Codex,
            CODEX,
            bounds(30_000, 5_000),
            Some(&cancel),
            SourceLimits::default()
        )),
        SourceUnavailable::Cancelled
    );
    canceller.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(gone(pid_in(&scene.root.join("leader.pid"))));
    assert!(gone(pid_in(&scene.root.join("child.pid"))));
    // A token already cancelled starts nothing.
    let python = fake(&scene, "touch \"$ROOT/started\"");
    assert_eq!(
        refused(load(
            &scene,
            &python,
            Host::Codex,
            CODEX,
            DetailBounds::default(),
            Some(&cancel),
            SourceLimits::default()
        )),
        SourceUnavailable::Cancelled
    );
    assert!(!scene.root.join("started").exists());
}

#[test]
fn a_reader_that_cannot_start_or_a_home_that_is_gone_leaves_nothing_behind() {
    let scene = scene();
    let before = tree(&scene.root);
    let missing: OsString = scene.root.join("no-such-python").into_os_string();
    assert_eq!(
        refused(load_codex(&scene, &missing)),
        SourceUnavailable::Reader(DetailFailure::Start)
    );
    let gone_home = Scene {
        _temp: tempfile::TempDir::new().unwrap(),
        root: scene.root.clone(),
        home: scene.root.join("never-created"),
    };
    let python = fake(&scene, "touch \"$ROOT/started\"");
    assert_eq!(
        refused(load_codex(&gone_home, &python)),
        SourceUnavailable::Missing
    );
    assert!(!scene.root.join("started").exists());
    fs::remove_file(scene.root.join("python3")).unwrap();
    assert_eq!(tree(&scene.root), before, "no file was left anywhere");
}

/// Quote a Unix path for the launcher's shell without losing non-UTF-8 bytes.
fn shell_path(path: &Path) -> Vec<u8> {
    let mut quoted = vec![b'\''];
    for &byte in path.as_os_str().as_bytes() {
        if byte == b'\'' {
            quoted.extend_from_slice(b"'\\''");
        } else {
            quoted.push(byte);
        }
    }
    quoted.push(b'\'');
    quoted
}

/// The reader clears its environment, so set TMPDIR inside a test-owned
/// interpreter launcher rather than in the Rust process or production code.
fn isolated_python(root: &Path, python: &OsString, snapshots: &Path) -> OsString {
    let launcher = root.join("python launcher");
    let mut script = b"#!/bin/sh\nTMPDIR=".to_vec();
    script.extend(shell_path(snapshots));
    script.extend_from_slice(b"\nexport TMPDIR\nexec ");
    script.extend(shell_path(Path::new(python)));
    script.extend_from_slice(b" \"$@\"\n");
    fs::write(&launcher, script).unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
    launcher.into_os_string()
}

/// Every entry in this test's own reader snapshot directory. Failure to read
/// it is a failed check, and even an unexpected non-snapshot entry is refused.
fn snapshot_names(dir: &Path) -> Vec<OsString> {
    let mut names: Vec<OsString> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    names
}

/// The ordinary export's records for one session, through the same parser.
fn ordinary_records(
    python: &OsString,
    producer: &PinnedProducer,
    host: Host,
    home: &Path,
    native: &str,
) -> Vec<xt_ingest::canonical::ParsedRecord> {
    let (stdout, handle) = spawn_reader(python, producer, host, home, None).unwrap();
    let mut events = StreamEvents::new(host, SessionSource::ReadersCli);
    let mut current = None;
    let mut records = Vec::new();
    for line in std::io::BufRead::lines(stdout) {
        match events.push(&line.unwrap()).unwrap() {
            Some(StreamEvent::Session(header)) => current = Some(header.native_session_id),
            Some(StreamEvent::Record(record)) if current.as_deref() == Some(native) => {
                records.push(*record)
            }
            _ => {}
        }
    }
    assert!(handle.finish().unwrap().complete);
    records
}

/// Lay F18's synthetic native files out under `home` with the existing
/// conformance harness.
fn materialize(python: &OsString, plugin_root: &std::ffi::OsStr, home: &Path) {
    let root = repo();
    let materialized = Command::new(python)
        .env_remove("PYTHONOPTIMIZE")
        .arg(root.join("scripts/conformance/test-reader-stream.py"))
        .arg("--plugin-root")
        .arg(plugin_root)
        .arg("--pin")
        .arg(root.join(".plugin-pin"))
        .arg("--fixtures")
        .arg(root.join("fixtures"))
        .arg("--materialize")
        .arg(home)
        .args(["--only", "F18"])
        .output()
        .expect("materialize the fixture home");
    assert!(
        materialized.status.success(),
        "{}",
        String::from_utf8_lossy(&materialized.stderr)
    );
}

#[test]
fn conformance_exact_detail() {
    let Some(plugin_root) = std::env::var_os("AGENT_PLUGINS_DIR") else {
        println!("SKIP conformance_exact_detail: set AGENT_PLUGINS_DIR to the pinned plugin root");
        return;
    };
    let named = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    let Ok(python) = resolve_python(Some(named.as_os_str()), None) else {
        println!("SKIP conformance_exact_detail: Python 3.10+ is unavailable");
        return;
    };
    let root = repo();
    let temp = tempfile::TempDir::new().unwrap();
    let temp_root = temp.path().canonicalize().unwrap();
    let home = temp_root.join("home");
    fs::create_dir_all(&home).unwrap();
    let snapshot_dir = temp_root.join("reader's snapshots");
    fs::create_dir(&snapshot_dir).unwrap();
    let python = isolated_python(&temp_root, &python, &snapshot_dir);
    // Exercise shell quoting and Python's actual tempfile choice with the
    // same cleared environment as the production reader command.
    let probe = Command::new(&python)
        .env_clear()
        .args([
            "-c",
            "import os,sys,tempfile\n\
             with tempfile.TemporaryDirectory(prefix='native-reader-') as path:\n\
             \x20sys.stdout.buffer.write(os.fsencode(os.path.dirname(path)))",
        ])
        .output()
        .unwrap();
    assert!(probe.status.success(), "{probe:?}");
    assert_eq!(probe.stdout, snapshot_dir.as_os_str().as_bytes());
    let snapshots = snapshot_names(&snapshot_dir);
    assert!(snapshots.is_empty(), "the probe left no staged snapshot");
    materialize(&python, &plugin_root, &home);
    // The store gains a committed write-ahead log that no checkpoint has
    // folded in: a read that opened it would have to recover the log, and one
    // that fell back to anything else would answer with an older session.
    let store = fs::read_dir(home.join(".cursor/chats"))
        .unwrap()
        .flat_map(|chat| fs::read_dir(chat.unwrap().path()).unwrap())
        .map(|session| session.unwrap().path().join("store.db"))
        .find(|path| path.is_file())
        .expect("F18 holds a Cursor store");
    let built = Command::new(&python)
        .args([
            "-c",
            "import os,sqlite3,sys\n\
             con=sqlite3.connect(sys.argv[1])\n\
             con.execute('PRAGMA journal_mode=WAL')\n\
             con.execute('PRAGMA wal_autocheckpoint=0')\n\
             con.execute('CREATE TABLE wal_only(x)')\n\
             con.execute(\"INSERT INTO wal_only VALUES ('WAL_ONLY')\")\n\
             con.commit()\n\
             os._exit(0)",
        ])
        .arg(&store)
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    assert!(store.with_file_name("store.db-wal").is_file());
    let _ = fs::remove_file(store.with_file_name("store.db-shm"));
    let expected: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("fixtures/F18/input/native/native.json")).unwrap(),
    )
    .unwrap();
    let wanted: Vec<(Host, &str, u64)> = expected["expected_headers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|want| {
            (
                match want["host"].as_str().unwrap() {
                    "codex" => Host::Codex,
                    _ => Host::Cursor,
                },
                want["native_session_id"].as_str().unwrap(),
                want["records"].as_u64().unwrap(),
            )
        })
        .collect();
    let store_id = store
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    // The exact candidate: the bundle as the app ships it, verified by object
    // identity. Checkout mode also runs the independently verified checkout.
    let bundled = producer();
    let checkout = if conformance_source::bundle_mode() {
        conformance_source::source(&root, Path::new(&plugin_root))
            .producer()
            .expect("the supplied bundle verifies");
        println!(
            "exact-detail bundle mode; independent checkout comparison requires checkout mode"
        );
        None
    } else {
        Some(
            conformance_source::source(&root, Path::new(&plugin_root))
                .producer()
                .expect("the checkout is at the pinned commit"),
        )
    };
    let producers: Vec<_> = std::iter::once(&bundled).chain(checkout.as_ref()).collect();
    let exact = |producer: &PinnedProducer, home: &Path, host: Host, native: &str| {
        load_session_source_with(
            &SessionSourceRequest {
                home,
                host,
                native_session_id: native,
                indexed: &[],
                cancel: None,
                limits: SourceLimits::default(),
            },
            Some(&DetailReader {
                python: &python,
                producer,
                bounds: DetailBounds::default(),
            }),
        )
    };
    let before = tree(&home);
    let mut loaded_sessions = 0;
    for producer in &producers {
        for &(host, native, records) in &wanted {
            let outcome = exact(producer, &home, host, native);
            if host == Host::Cursor && native == store_id {
                assert_eq!(
                    outcome,
                    SessionSourceOutcome::Unavailable(SourceUnavailable::Reader(
                        DetailFailure::Refused(DetailRefusal::StoreUnsupported)
                    )),
                    "{native}"
                );
                continue;
            }
            let SessionSourceOutcome::Loaded(loaded) = outcome else {
                panic!("{native}: {outcome:?}");
            };
            assert_eq!(loaded.records.len() as u64, records, "{native}");
            assert_eq!(loaded.dropped_records, 0, "{native}");
            assert!(loaded.sources[0].path.starts_with(&home), "{native}");
            loaded_sessions += 1;
        }
    }
    assert_eq!(
        loaded_sessions,
        2 * producers.len(),
        "two JSONL sessions, from each producer"
    );
    // Exact reads alone, of every session and of the store with its log,
    // changed neither the home nor the reader temporary directory: no journal
    // recovery, no shared-memory file, no copy, no source write, no snapshot.
    assert_eq!(tree(&home), before, "the exact reads changed the home");
    assert!(!store.with_file_name("store.db-shm").exists());
    assert_eq!(
        snapshot_names(&snapshot_dir),
        snapshots,
        "no reader snapshot was staged"
    );
    // And record for record, what the exact reads return is what the ordinary
    // export produces for the same session.
    for &(host, native, _) in &wanted {
        if host == Host::Cursor && native == store_id {
            continue;
        }
        let SessionSourceOutcome::Loaded(loaded) = exact(&bundled, &home, host, native) else {
            panic!("{native}");
        };
        assert_eq!(
            loaded.records,
            ordinary_records(&python, &bundled, host, &home, native),
            "{native}"
        );
    }
    // A paginated Codex session is its whole history — every rollout, and the
    // prefix a continuation inherits counted once. Checkout mode uses its
    // own history fixture; bundle mode supplies equivalent synthetic data.
    let paged = tempfile::TempDir::new().unwrap();
    let paged_home = paged.path().canonicalize().unwrap();
    let checkout_root = Path::new(&plugin_root).join("../..");
    let fixture_script = if conformance_source::bundle_mode() {
        "import runpy,sys\nfrom pathlib import Path\n\
         fixture=runpy.run_path(sys.argv[1])\n\
         fixture['fixture'](Path(sys.argv[2]))\nprint(fixture['SID'])"
    } else {
        "import sys\nfrom pathlib import Path\n\
         sys.path[:0]=[sys.argv[1]+'/tests', sys.argv[1]+'/plugins/memhub/scripts']\n\
         import codex_history_test as history\n\
         history.fixture(Path(sys.argv[2]))\nprint(history.SID)"
    };
    let fixture_path = if conformance_source::bundle_mode() {
        root.join("scripts/conformance/paginated-fixture.py")
    } else {
        checkout_root
    };
    let built = Command::new(&python)
        .env_remove("PYTHONOPTIMIZE")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .args(["-c", fixture_script])
        .arg(&fixture_path)
        .arg(&paged_home)
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    let paged_id = String::from_utf8(built.stdout).unwrap().trim().to_owned();
    let paged_before = tree(&paged_home);
    let SessionSourceOutcome::Loaded(loaded) = exact(&bundled, &paged_home, Host::Codex, &paged_id)
    else {
        panic!("the paginated session loads");
    };
    assert_eq!(
        tree(&paged_home),
        paged_before,
        "the rollouts are unchanged"
    );
    let identities: std::collections::BTreeSet<_> = loaded
        .records
        .iter()
        .map(|record| record.canonical.uuid.clone())
        .collect();
    assert_eq!(
        loaded.records.len(),
        6,
        "two rollouts, the shared prefix once"
    );
    assert_eq!(identities.len(), 6);
    assert_eq!(
        loaded.records,
        ordinary_records(&python, &bundled, Host::Codex, &paged_home, &paged_id)
    );
    // Configured roots that are symlinks to elsewhere — the layout the reader
    // supports — report sources where the roots lead. The exact reads load
    // them, as the ordinary export does, and leave both sides untouched.
    let linked = tempfile::TempDir::new().unwrap();
    let linked_root = linked.path().canonicalize().unwrap();
    let linked_home = linked_root.join("home");
    fs::create_dir_all(&linked_home).unwrap();
    materialize(&python, &plugin_root, &linked_home);
    let elsewhere = linked_root.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    for (relative, target) in [
        (".codex/sessions", "codex-sessions"),
        (".cursor/projects", "cursor-projects"),
    ] {
        let spelled = linked_home.join(relative);
        fs::rename(&spelled, elsewhere.join(target)).unwrap();
        std::os::unix::fs::symlink(elsewhere.join(target), &spelled).unwrap();
    }
    let linked_before = (tree(&linked_home), tree(&elsewhere));
    let mut linked_sessions = 0;
    for &(host, native, records) in &wanted {
        if host == Host::Cursor && native == store_id {
            continue;
        }
        let SessionSourceOutcome::Loaded(loaded) = exact(&bundled, &linked_home, host, native)
        else {
            panic!("{native} under a symlinked root");
        };
        assert_eq!(loaded.records.len() as u64, records, "{native}");
        assert!(loaded.sources[0].path.starts_with(&elsewhere), "{native}");
        linked_sessions += 1;
    }
    assert_eq!(linked_sessions, 2);
    assert_eq!(
        (tree(&linked_home), tree(&elsewhere)),
        linked_before,
        "exact reads under symlinked roots changed nothing"
    );
    for &(host, native, _) in &wanted {
        if host == Host::Cursor && native == store_id {
            continue;
        }
        let SessionSourceOutcome::Loaded(loaded) = exact(&bundled, &linked_home, host, native)
        else {
            panic!("{native}");
        };
        assert_eq!(
            loaded.records,
            ordinary_records(&python, &bundled, host, &linked_home, native),
            "{native} under a symlinked root"
        );
    }
    println!(
        "exact detail conformed at {} (memhub {}): {loaded_sessions} sessions, store refused",
        bundled.commit, bundled.plugin_version
    );
}
