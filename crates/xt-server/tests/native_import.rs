//! The headless `import-native` command: a JSON report on stdout, exit 0 only
//! when every requested host imported completely, 2 for an explicit gap.
use serde_json::Value;
use std::{fs, process::Command};

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
