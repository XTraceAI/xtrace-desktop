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
    let request = ImportRequest {
        home: &home,
        hosts: &[Host::Codex, Host::Cursor],
        pin: &root.join(".plugin-pin"),
        plugin_root: Some(Path::new(&plugin_root)),
        python: Some(&python),
        observed_at: 1_788_782_400_000,
    };
    let report = import_native(&mut store, &request);
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
    let again = import_native(&mut store, &request);
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
            ..request
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
