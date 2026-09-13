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
