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
    use xt_ingest::native::{
        HostStatus, ImportRequest, ProducerSource, SessionOutcome, import_native,
    };
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
    let producer = ProducerSource::Checkout {
        pin: root.join(".plugin-pin"),
        plugin_root: Some(PathBuf::from(&plugin_root)),
    };
    let request_now = || ImportRequest {
        home: &home,
        hosts: &[Host::Codex, Host::Cursor],
        producer: &producer,
        python: Some(&python),
        observed_at: now_ms(),
        cancel: None,
    };
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
    // Every scan reads every session again through the pinned producer, with
    // no cutoff: a repeated scan reports complete coverage and adds nothing,
    // since records dedupe by UUID.
    let host_of = |report: &xt_ingest::native::ImportReport, host: Host| {
        report
            .hosts
            .iter()
            .find(|candidate| candidate.host == host)
            .unwrap()
            .clone()
    };
    let nothing_new = |host: &xt_ingest::native::HostReport| {
        host.status == HostStatus::Complete
            && host.sessions.iter().all(|session| {
                session.outcome
                    == SessionOutcome::Imported {
                        records_new: 0,
                        records_enriched: 0,
                    }
            })
    };
    let again = import_native(&mut store, &request_now());
    assert!(again.complete(), "{again:?}");
    for (host, first) in again.hosts.iter().zip(&report.hosts) {
        assert!(nothing_new(host), "{host:?}");
        assert_eq!(host.sessions.len(), first.sessions.len());
        let detail = host.detail.as_deref().unwrap();
        assert!(
            detail.contains("pinned producer") && !detail.contains("since"),
            "{detail}"
        );
    }
    // A Codex session restored into the tree with an old clock is read like
    // any other: the whole host is read on every scan.
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
    let set_clock = |path: &Path, clock: std::time::SystemTime| {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(clock))
            .unwrap();
    };
    set_clock(&restored, old_clock);
    let codex = host_of(&import_native(&mut store, &request_now()), Host::Codex);
    assert_eq!(codex.status, HostStatus::Complete, "{codex:?}");
    assert_eq!(codex.sessions.len(), 2, "{codex:?}");
    let restored_session = codex
        .sessions
        .iter()
        .find(|session| {
            session.native_session_id.as_deref() == Some("00000000-0000-4000-8000-000000000191")
        })
        .unwrap();
    assert!(
        matches!(restored_session.outcome, SessionOutcome::Imported { records_new, .. } if records_new > 0),
        "the restored session is read and indexed: {codex:?}"
    );
    fs::remove_file(&restored).unwrap();
    // The original given an old clock, then rewritten in place with the same
    // bytes and that clock put back (a synchronization tool's replacement):
    // read again each time, nothing new either time.
    set_clock(&original, old_clock);
    let aged = host_of(&import_native(&mut store, &request_now()), Host::Codex);
    assert!(nothing_new(&aged), "{aged:?}");
    assert_eq!(aged.sessions.len(), 1, "{aged:?}");
    let bytes = fs::read(&original).unwrap();
    fs::write(&original, &bytes).unwrap();
    set_clock(&original, old_clock);
    let rewritten = host_of(&import_native(&mut store, &request_now()), Host::Codex);
    assert!(nothing_new(&rewritten), "{rewritten:?}");
    assert_eq!(rewritten.sessions.len(), 1, "{rewritten:?}");
    // A store-backed Cursor session's `meta.json` rewritten in place with its
    // clock put back, and a hook state pin appearing beside the session: each
    // is read through the same whole scan, nothing new.
    let chat = home.join(".cursor/chats/19ee0000fixture0/00000000-0000-4000-8000-000000000183");
    let meta = chat.join("meta.json");
    let meta_bytes = fs::read(&meta).unwrap();
    let meta_clock = fs::metadata(&meta).unwrap().modified().unwrap();
    fs::write(&meta, &meta_bytes).unwrap();
    set_clock(&meta, meta_clock);
    let sidecar = host_of(&import_native(&mut store, &request_now()), Host::Cursor);
    assert!(nothing_new(&sidecar), "{sidecar:?}");
    assert!(
        sidecar.sessions.iter().any(|session| {
            session.native_session_id.as_deref() == Some("00000000-0000-4000-8000-000000000183")
        }),
        "{sidecar:?}"
    );
    let pins = home.join(".config/memhub-plugin/cursorflush");
    fs::create_dir_all(&pins).unwrap();
    fs::write(
        pins.join("00000000-0000-4000-8000-000000000183.json"),
        b"{}",
    )
    .unwrap();
    let pinned = host_of(&import_native(&mut store, &request_now()), Host::Cursor);
    assert!(nothing_new(&pinned), "{pinned:?}");
    assert_eq!(pinned.sessions.len(), sidecar.sessions.len());
    fs::remove_dir_all(home.join(".config")).unwrap();
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

/// The bundled reader sources the app ships are byte for byte the pinned
/// commit's scripts tree, notices included, and read the F18 fixture home to
/// the same index as the verified checkout, in place, with no checkout or Git.
#[test]
fn conformance_bundled_readers() {
    use xt_ingest::native::{
        HostStatus, ImportRequest, ProducerSource, import_native, readers_cli::read_pin,
    };
    use xt_store::{Host, Store};
    let Some(plugin_root) = std::env::var_os("AGENT_PLUGINS_DIR") else {
        println!(
            "SKIP conformance_bundled_readers: set AGENT_PLUGINS_DIR to the pinned plugin root"
        );
        return;
    };
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    if Command::new(&python).arg("--version").output().is_err() {
        println!("SKIP conformance_bundled_readers: Python 3 is unavailable");
        return;
    }
    let root = repo_root();
    let pin = read_pin(&root.join(".plugin-pin")).unwrap();
    let bundle = root.join("vendor/agent-plugins");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&plugin_root)
            .args(args)
            .output()
            .expect("git runs in the pinned checkout");
        assert!(output.status.success(), "git {args:?}");
        output.stdout
    };
    // Every committed file below the scripts tree, and only those, with the
    // committed bytes; the notices as committed too.
    let scripts = format!("{}/scripts", pin.plugin_root);
    // The checkout root may be the plugin root: paths are asked for from the
    // tree's root, as the pin names them.
    let listed = String::from_utf8(git(&[
        "ls-tree",
        "-r",
        "--full-tree",
        "--name-only",
        &pin.commit,
        &scripts,
    ]))
    .unwrap();
    let mut committed = listed.lines().map(str::to_owned).collect::<Vec<_>>();
    committed.sort();
    fn vendored(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "__pycache__" {
                continue;
            }
            let path = format!("{prefix}/{name}");
            if entry.file_type().unwrap().is_dir() {
                vendored(&entry.path(), &path, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut shipped = Vec::new();
    vendored(&bundle.join(&scripts), &scripts, &mut shipped);
    shipped.sort();
    assert_eq!(
        shipped, committed,
        "the bundle holds exactly the committed scripts"
    );
    for path in committed
        .iter()
        .chain(["LICENSE".to_owned(), "NOTICE".to_owned()].iter())
    {
        assert_eq!(
            fs::read(bundle.join(path)).unwrap(),
            git(&["show", &format!("{}:{path}", pin.commit)]),
            "{path} differs from the pinned commit"
        );
    }
    println!(
        "bundled readers verified against {} (memhub {}): {} files",
        pin.commit,
        pin.plugin_version,
        committed.len()
    );
    // The same fixture home, read by the bundle and by the checkout, yields
    // the same sessions and counts, and leaves the sources untouched.
    let temp = std::env::temp_dir().join(format!("xtrace-bundle-{}", std::process::id()));
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
    let mut sources = std::collections::BTreeMap::new();
    fn hashes(dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                hashes(&path, out);
            } else {
                out.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    hashes(&home, &mut sources);
    let run = |producer: ProducerSource, index: &str| {
        let mut store = Store::open(temp.join(index)).unwrap();
        let report = import_native(
            &mut store,
            &ImportRequest {
                home: &home,
                hosts: &[Host::Codex, Host::Cursor],
                producer: &producer,
                python: Some(&python),
                observed_at: now_ms(),
                cancel: None,
            },
        );
        assert!(report.complete(), "{report:?}");
        (report, store.counts().unwrap())
    };
    let (from_bundle, bundle_counts) = run(
        ProducerSource::Bundle {
            pin: pin.clone(),
            root: bundle.clone(),
        },
        "bundle.sqlite",
    );
    let (from_checkout, checkout_counts) = run(
        ProducerSource::Checkout {
            pin: root.join(".plugin-pin"),
            plugin_root: Some(PathBuf::from(&plugin_root)),
        },
        "checkout.sqlite",
    );
    for (bundled, checked) in from_bundle.hosts.iter().zip(&from_checkout.hosts) {
        assert_eq!(bundled.status, HostStatus::Complete);
        assert_eq!(bundled.host, checked.host);
        assert_eq!(
            bundled.sessions,
            checked.sessions,
            "{}",
            bundled.host.as_str()
        );
        let detail = bundled.detail.as_deref().unwrap();
        assert!(detail.contains(&pin.commit), "{detail}");
    }
    assert_eq!(bundle_counts.sessions, checkout_counts.sessions);
    assert_eq!(bundle_counts.records, checkout_counts.records);
    assert_eq!(bundle_counts.usage_rows, checkout_counts.usage_rows);
    assert!(bundle_counts.sessions > 0);
    let mut after = std::collections::BTreeMap::new();
    hashes(&home, &mut after);
    assert_eq!(sources, after, "the sources are unchanged");
    // The bundle's readers ran from the bundle itself, not from an export.
    let producer = ProducerSource::Bundle {
        pin,
        root: bundle.clone(),
    }
    .producer()
    .unwrap();
    assert!(producer.script.starts_with(&bundle));
    let _ = fs::remove_dir_all(&temp);
}
