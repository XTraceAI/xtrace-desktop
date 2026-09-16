use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn validate_reports_all_entries_and_both_asserted_fixtures() {
    let output = run(&["fixture-validate"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        report
            .lines()
            .filter(|line| line.contains("SKELETON"))
            .count(),
        19
    );
    assert_eq!(
        report
            .lines()
            .filter(|line| line.contains("POPULATED"))
            .count(),
        2
    );
    assert!(report.contains("F21: SKELETON"));
    assert!(report.contains("F3: POPULATED"));
    assert!(report.contains(
        "21 entries structurally valid; 2 populated/asserted; 19 skeleton/unimplemented"
    ));
}

#[test]
fn db_command_materializes_sqlite_and_rejects_overwrite_or_skeleton() {
    let directory = TempDir::new().unwrap();
    let file = directory.path().join("F1.sqlite");
    let path = file.to_str().unwrap();
    let output = run(&["fixture-db", "F1", "--out", path]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let original = fs::read(&file).unwrap();
    assert!(original.starts_with(b"SQLite format 3\0"));
    assert!(!run(&["fixture-db", "F1", "--out", path]).status.success());
    assert_eq!(fs::read(&file).unwrap(), original);
    let skeleton = directory.path().join("F2.sqlite");
    let output = run(&["fixture-db", "F2", "--out", skeleton.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unimplemented"));
    assert!(!skeleton.exists());
}

#[test]
fn export_uses_shared_canonical_data_and_preserves_skeleton_status() {
    let output = run(&["fixture-export", "F1"]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["sessions"][0]["records"].as_array().unwrap().len(),
        25
    );
    assert_eq!(value["expected"]["M-04"]["total_tokens"], 1100);
    let directory = TempDir::new().unwrap();
    let file = directory.path().join("F16.json");
    let output = run(&["fixture-export", "F16", "--out", file.to_str().unwrap()]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(value["manifest"]["status"], "skeleton");
    assert_eq!(value["snapshots"].as_object().unwrap().len(), 5);
    assert!(value["expected"].as_object().unwrap().is_empty());
    assert!(
        !run(&["fixture-export", "F16", "--out", file.to_str().unwrap()])
            .status
            .success()
    );
}

#[test]
fn bad_commands_fail_instead_of_reporting_placeholder_success() {
    for args in [
        vec!["unknown"],
        vec!["fixture-db"],
        vec!["fixture-db", "F1"],
        vec!["fixture-export", "F01"],
        vec!["fixture-export", "F22"],
        vec!["fixture-validate", "--out", "ignored"],
        vec!["fixture-validate", "--catalog"],
        vec!["fixture-db", "F1", "--out", "--catalog"],
        vec!["fixture-validate", "--catalog", "one", "--catalog", "two"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn missing_catalog_fails_with_location() {
    let directory = TempDir::new().unwrap();
    let output = run(&[
        "fixture-validate",
        "--catalog",
        directory.path().to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("F1"));
}
