//! Pinned native reader stream conformance. The exact reviewed producer named by
//! `.plugin-pin` runs over the fixture catalog's synthetic native files; its
//! session headers and canonical records must satisfy the fixture expectations,
//! the committed contract golden, and this crate's own canonical parser.
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use xt_ingest::canonical::{Parsed, SourceContext, parse_with_context};

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn evidence_path() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "xtrace-reader-stream-{}-{stamp}.json",
        std::process::id()
    ))
}

#[test]
fn conformance_native_reader_stream() {
    let Some(plugin_root) = std::env::var_os("AGENT_PLUGINS_DIR") else {
        println!(
            "SKIP conformance_native_reader_stream: set AGENT_PLUGINS_DIR to the pinned plugin root"
        );
        return;
    };
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    if Command::new(&python).arg("--version").output().is_err() {
        println!("SKIP conformance_native_reader_stream: Python 3 is unavailable");
        return;
    }
    let root = repo_root();
    let evidence = evidence_path();
    let output = Command::new(python)
        // Assertions carry the harness contracts; an inherited -O must not strip them.
        .env_remove("PYTHONOPTIMIZE")
        .arg(root.join("scripts/conformance/test-reader-stream.py"))
        .arg("--plugin-root")
        .arg(plugin_root)
        .arg("--pin")
        .arg(root.join(".plugin-pin"))
        .arg("--fixtures")
        .arg(root.join("fixtures"))
        .arg("--output")
        .arg(&evidence)
        .arg("--self-test")
        .output()
        .expect("Run the pinned reader stream conformance harness");
    print!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "Pinned reader stream conformance failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(&evidence).expect("harness evidence");
    let _ = fs::remove_file(&evidence);
    let report: Value = serde_json::from_str(&text).expect("harness evidence is JSON");
    let runs = report["runs"].as_array().expect("runs");
    assert!(
        !runs.is_empty(),
        "the harness must exercise at least one fixture"
    );
    let mut total_records = 0usize;
    for run in runs {
        let host = run["host"].as_str().expect("host");
        let mut context = SourceContext::default();
        let mut headers = 0usize;
        let mut records = 0usize;
        for line in run["lines"].as_array().expect("lines") {
            let line = line.as_str().expect("line");
            let row: Value = serde_json::from_str(line).expect("stream line is JSON");
            if row["type"] == "session" {
                headers += 1;
                // The header supplies the import context the later native-source
                // ingestion will hand to this parser for the records that follow.
                context = SourceContext {
                    conversation_id: row["conversation_id"].as_str().map(str::to_owned),
                    native_session_id: row["native_session_id"].as_str().map(str::to_owned),
                    source_platform: Some(host.to_owned()),
                    source_surface: row["source_surface"].as_str().map(str::to_owned),
                    started_at: row["started_at"].as_str().map(str::to_owned),
                    source: None,
                };
                assert!(
                    matches!(parse_with_context(line, &context), Ok(Parsed::Inert)),
                    "a session header must stay inert for the canonical parser"
                );
                continue;
            }
            let Parsed::Record(record) =
                parse_with_context(line, &context).expect("canonical record parses")
            else {
                panic!("every non-header line must be a canonical record");
            };
            assert!(record.canonical.uuid.is_some(), "records carry a UUID");
            assert!(record.canonical.timestamp.is_some() || host == "cursor");
            assert_eq!(
                record.context, context,
                "records inherit their header context"
            );
            records += 1;
        }
        assert_eq!(headers, run["headers"].as_u64().expect("headers") as usize);
        assert_eq!(records, run["records"].as_u64().expect("records") as usize);
        total_records += records;
    }
    assert!(
        total_records > 0,
        "full exports must produce canonical records"
    );
}

/// The complete native import path with the exact pinned producer: F18's
/// synthetic Codex and Cursor inputs are laid out under a disposable home, the
/// pinned readers run with no plugin configuration, credentials or network,
/// every session lands with its identity and counts, a second run adds nothing
/// and the source bytes are unchanged.
#[test]
fn conformance_native_import() {
    use std::collections::BTreeMap;
    use xt_ingest::native::{HostStatus, ImportRequest, SessionOutcome, import_native};
    use xt_store::{Host, Store};
    let Some(plugin_root) = std::env::var_os("AGENT_PLUGINS_DIR") else {
        println!("SKIP conformance_native_import: set AGENT_PLUGINS_DIR to the pinned plugin root");
        return;
    };
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    if Command::new(&python).arg("--version").output().is_err() {
        println!("SKIP conformance_native_import: Python 3 is unavailable");
        return;
    }
    let root = repo_root();
    let temp = std::env::temp_dir().join(format!("xtrace-native-import-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    let home = temp.join("home");
    fs::create_dir_all(&home).unwrap();
    let materialized = Command::new(&python)
        .env_remove("PYTHONOPTIMIZE")
        .arg(root.join("scripts/conformance/test-reader-stream.py"))
        .args(["--plugin-root"])
        .arg(&plugin_root)
        .arg("--pin")
        .arg(root.join(".plugin-pin"))
        .arg("--fixtures")
        .arg(root.join("fixtures"))
        .arg("--materialize")
        .arg(&home)
        .args(["--only", "F18"])
        .output()
        .expect("materialize the fixture home");
    assert!(
        materialized.status.success(),
        "{}",
        String::from_utf8_lossy(&materialized.stderr)
    );
    fn hashes(dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                hashes(&path, out);
            } else {
                out.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    let mut before = BTreeMap::new();
    hashes(&home, &mut before);
    let expected: Value = serde_json::from_str(
        &fs::read_to_string(root.join("fixtures/F18/input/native/native.json")).unwrap(),
    )
    .unwrap();
    let mut store = Store::open(temp.join("index.sqlite")).unwrap();
    let pin = root.join(".plugin-pin");
    let request_now = || ImportRequest {
        home: &home,
        hosts: &[Host::Codex, Host::Cursor],
        pin: &pin,
        plugin_root: Some(Path::new(&plugin_root)),
        python: Some(&python),
        observed_at: now_ms(),
    };
    // A session written within the last 2 s is never covered by a cutoff (a
    // coarse clock could hide a rewrite), so runs that expect the cutoff wait
    // for the source set to settle first.
    let settle = || std::thread::sleep(std::time::Duration::from_millis(2_100));
    settle();
    let report = import_native(&mut store, &request_now());
    println!("{}", serde_json::to_string(&report).unwrap());
    assert!(report.complete(), "{report:?}");
    for host in &report.hosts {
        assert_eq!(host.status, HostStatus::Complete);
        assert!(host.detail.as_deref().unwrap().contains("pinned producer"));
        let want = expected["expected_headers"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["host"] == host.host.as_str())
            .collect::<Vec<_>>();
        assert_eq!(host.sessions.len(), want.len());
        for (session, want) in host.sessions.iter().zip(want) {
            assert_eq!(
                session.native_session_id.as_deref(),
                want["native_session_id"].as_str()
            );
            assert_eq!(
                session.source_surface,
                want["source_surface"].as_str().map(str::to_owned)
            );
            assert_eq!(
                session.outcome,
                SessionOutcome::Imported {
                    records_new: want["records"].as_u64().unwrap() as usize,
                    records_enriched: 0
                }
            );
            let stored = store
                .session(session.conversation_id.as_deref().unwrap())
                .unwrap()
                .unwrap();
            assert_eq!(stored.meta.title, None);
            assert!(
                store
                    .records(&stored.meta.session_id)
                    .unwrap()
                    .iter()
                    .all(|r| r.content_json.is_none())
            );
        }
    }
    settle();
    let again = import_native(&mut store, &request_now());
    assert!(again.complete());
    // The first scan covered every session, so the second asks the pinned
    // producer only for sessions modified since that scan started: it still
    // reports complete coverage and adds nothing.
    for host in &again.hosts {
        assert!(
            host.detail
                .as_deref()
                .is_some_and(|detail| detail.contains("sessions modified since")),
            "{host:?}"
        );
    }
    // A Codex session restored into the tree with an old clock, older than
    // the cutoff: the cutoff would skip it, so the inventory check forces a
    // full scan and the session is read. Once it is gone again, the cutoff is
    // safe once more.
    let rollout = home.join(".codex/sessions/2026/09/07");
    let original = fs::read_dir(&rollout)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .unwrap();
    let restored = rollout.join(
        original
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .replace("000000000181", "000000000191"),
    );
    fs::write(
        &restored,
        fs::read_to_string(&original)
            .unwrap()
            .replace("000000000181", "000000000191"),
    )
    .unwrap();
    let old_clock =
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    fs::File::options()
        .write(true)
        .open(&restored)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old_clock))
        .unwrap();
    let with_restored = import_native(&mut store, &request_now());
    let codex = with_restored
        .hosts
        .iter()
        .find(|host| host.host == Host::Codex)
        .unwrap();
    assert!(
        codex
            .detail
            .as_deref()
            .is_some_and(|detail| !detail.contains("sessions modified since")),
        "a session older than the cutoff that the scan never covered forces a full scan: {codex:?}"
    );
    assert!(
        codex.sessions.iter().any(|session| {
            session.native_session_id.as_deref() == Some("00000000-0000-4000-8000-000000000191")
                && !matches!(session.outcome, SessionOutcome::Skipped { .. })
        }),
        "the restored session is read: {codex:?}"
    );
    fs::remove_file(&restored).unwrap();
    // The original given an old clock, older than the cutoff: its stamp is
    // new to the inventory, so one more full scan reads it; after that the
    // cutoff covers it and the producer skips it.
    let aged_clock =
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_100);
    let age = |path: &Path| {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(aged_clock))
            .unwrap();
    };
    age(&original);
    let codex_of = |report: &xt_ingest::native::ImportReport| {
        report
            .hosts
            .iter()
            .find(|host| host.host == Host::Codex)
            .unwrap()
            .clone()
    };
    let aged = codex_of(&import_native(&mut store, &request_now()));
    assert_eq!(aged.status, HostStatus::Complete, "{aged:?}");
    assert!(
        aged.detail
            .as_deref()
            .is_some_and(|detail| !detail.contains("sessions modified since")),
        "a session with a clock the inventory does not know is read in a full scan: {aged:?}"
    );
    assert_eq!(aged.sessions.len(), 1, "{aged:?}");
    // That scan started within 2 s of the change, so its generation cannot
    // vouch for the session on a coarse clock: one more full scan, started
    // after the change settled, records a generation that can.
    settle();
    let settling = codex_of(&import_native(&mut store, &request_now()));
    assert_eq!(settling.status, HostStatus::Complete, "{settling:?}");
    assert!(
        settling
            .detail
            .as_deref()
            .is_some_and(|detail| !detail.contains("sessions modified since")),
        "an unsettled generation forces one more full scan: {settling:?}"
    );
    settle();
    let skipped = codex_of(&import_native(&mut store, &request_now()));
    assert_eq!(skipped.status, HostStatus::Complete, "{skipped:?}");
    assert!(
        skipped
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("sessions modified since")),
        "{skipped:?}"
    );
    assert!(
        skipped.sessions.is_empty(),
        "the covered old session is skipped behind the cutoff: {skipped:?}"
    );
    // Rewritten in place with the same bytes and the old clock put back (a
    // synchronization tool's replacement): the change time moved, so the
    // cutoff cannot cover it and a full scan reads it again.
    let bytes = fs::read(&original).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(&original, &bytes).unwrap();
    age(&original);
    let rewritten = codex_of(&import_native(&mut store, &request_now()));
    assert_eq!(rewritten.status, HostStatus::Complete, "{rewritten:?}");
    assert!(
        rewritten
            .detail
            .as_deref()
            .is_some_and(|detail| !detail.contains("sessions modified since")),
        "a clock-preserving rewrite forces a full scan: {rewritten:?}"
    );
    assert_eq!(rewritten.sessions.len(), 1, "{rewritten:?}");
    settle();
    assert!(
        codex_of(&import_native(&mut store, &request_now()))
            .detail
            .as_deref()
            .is_some_and(|detail| !detail.contains("sessions modified since")),
        "one more full scan records a settled generation"
    );
    settle();
    let after_removal = import_native(&mut store, &request_now());
    let codex = after_removal
        .hosts
        .iter()
        .find(|host| host.host == Host::Codex)
        .unwrap();
    assert_eq!(codex.status, HostStatus::Complete, "{codex:?}");
    assert!(
        codex
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("sessions modified since")),
        "every older session is covered again: {codex:?}"
    );
    assert!(
        again
            .hosts
            .iter()
            .flat_map(|h| &h.sessions)
            .all(|s| matches!(
                s.outcome,
                SessionOutcome::Imported {
                    records_new: 0,
                    records_enriched: 0,
                    ..
                }
            ))
    );
    // A store-backed Cursor session's `meta.json` rewritten in place with the
    // same bytes and its clock put back: the store itself is untouched and the
    // producer's clock for the session unchanged, but the sidecar's change
    // time moved, so the cutoff cannot cover the session and a full scan
    // reads it again; once settled, the cutoff covers it once more.
    let meta =
        home.join(".cursor/chats/19ee0000fixture0/00000000-0000-4000-8000-000000000183/meta.json");
    let meta_bytes = fs::read(&meta).unwrap();
    let meta_clock = fs::metadata(&meta).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(&meta, &meta_bytes).unwrap();
    fs::File::options()
        .write(true)
        .open(&meta)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(meta_clock))
        .unwrap();
    let cursor_of = |report: &xt_ingest::native::ImportReport| {
        report
            .hosts
            .iter()
            .find(|host| host.host == Host::Cursor)
            .unwrap()
            .clone()
    };
    let sidecar = cursor_of(&import_native(&mut store, &request_now()));
    assert_eq!(sidecar.status, HostStatus::Complete, "{sidecar:?}");
    assert!(
        sidecar
            .detail
            .as_deref()
            .is_some_and(|detail| !detail.contains("sessions modified since")),
        "a clock-preserving sidecar rewrite forces a full scan: {sidecar:?}"
    );
    assert!(
        sidecar.sessions.iter().any(|session| {
            session.native_session_id.as_deref() == Some("00000000-0000-4000-8000-000000000183")
                && !matches!(session.outcome, SessionOutcome::Skipped { .. })
        }),
        "the session behind the rewritten sidecar is read: {sidecar:?}"
    );
    settle();
    assert!(
        cursor_of(&import_native(&mut store, &request_now()))
            .detail
            .as_deref()
            .is_some_and(|detail| !detail.contains("sessions modified since")),
        "one more full scan records a settled generation"
    );
    settle();
    let covered = cursor_of(&import_native(&mut store, &request_now()));
    assert_eq!(covered.status, HostStatus::Complete, "{covered:?}");
    assert!(
        covered
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("sessions modified since")),
        "the rewritten sidecar is covered again: {covered:?}"
    );
    // A relative spelling of the same home reaches the readers anchored: they
    // change into the home and receive it as HOME, so a relative value would
    // resolve beneath itself and report a complete, empty import.
    let cwd = std::env::current_dir().unwrap();
    let relative = format!(
        "{}{}",
        "../".repeat(cwd.components().count() - 1),
        home.canonicalize()
            .unwrap()
            .to_string_lossy()
            .trim_start_matches('/')
    );
    assert!(Path::new(&relative).is_relative());
    let relative_run = import_native(
        &mut store,
        &ImportRequest {
            home: Path::new(&relative),
            ..request_now()
        },
    );
    assert!(relative_run.complete(), "{relative_run:?}");
    for (host, earlier) in relative_run.hosts.iter().zip(&report.hosts) {
        assert_eq!(host.sessions.len(), earlier.sessions.len(), "{host:?}");
        for session in &host.sessions {
            assert!(
                Path::new(session.path.as_deref().unwrap()).is_absolute(),
                "{session:?}"
            );
            assert_eq!(
                session.outcome,
                SessionOutcome::Imported {
                    records_new: 0,
                    records_enriched: 0
                },
                "already imported under the absolute spelling"
            );
        }
    }
    let mut after = BTreeMap::new();
    hashes(&home, &mut after);
    assert_eq!(
        after, before,
        "native source bytes and file set are unchanged"
    );
    let _ = fs::remove_dir_all(&temp);
}
