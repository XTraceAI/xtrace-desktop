use std::{fs, process::Command};

fn assert_shell_export_matches_golden(actual: &str, golden: &str) {
    let actual_metadata: serde_json::Value = serde_json::from_str(actual).unwrap();
    assert_eq!(
        actual_metadata["app_info"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(actual, golden_with_package_version(golden));
}

fn golden_with_package_version(golden: &str) -> String {
    // Fail closed if the committed AppInfo header changes shape. Never round-trip
    // the body: parsed floats are not round-trip exact under serde_json defaults.
    const PREFIX: &str = "{\n  \"app_info\": {\n";
    let (header, _) = golden
        .strip_prefix(PREFIX)
        .expect("golden must start with the AppInfo header")
        .split_once("\n  },\n")
        .expect("golden must delimit the AppInfo header exactly");
    assert_eq!(header.matches("\"version\"").count(), 1);
    let metadata: serde_json::Value = serde_json::from_str(&format!("{{\n{header}\n}}"))
        .expect("golden AppInfo header must be valid JSON");
    let version = metadata["version"]
        .as_str()
        .expect("golden AppInfo version must be a string");
    let token = format!("    \"version\": \"{version}\",\n");
    assert_eq!(header.matches(&token).count(), 1);
    let start = PREFIX.len() + header.find(&token).unwrap() + "    \"version\": \"".len();
    let mut expected = golden.to_owned();
    expected.replace_range(start..start + version.len(), env!("CARGO_PKG_VERSION"));
    expected
}

#[test]
fn generated_shell_export_uses_ipc_structs_and_rejects_skeletons() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["fixture-export", "F1", "--shell"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_shell_export_matches_golden(
        std::str::from_utf8(&output.stdout).unwrap(),
        include_str!("../../apps/desktop/ui/fixtures/F1.json"),
    );
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(actual["app_info"]["data_dir"], "fixture://F1");
    for id in ["F2", "F20"] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["fixture-export", id, "--shell"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

const SYNTHETIC_GOLDEN: &str = "{\n  \"app_info\": {\n    \"version\": \"0.0.0\",\n    \"fixture\": \"synthetic\"\n  },\n  \"nested\": {\"version\": \"0.0.0\"},\n  \"value\": 1.2500\n}\n";

fn synthetic_current_export() -> String {
    SYNTHETIC_GOLDEN.replacen(
        "    \"version\": \"0.0.0\",\n",
        &format!("    \"version\": \"{}\",\n", env!("CARGO_PKG_VERSION")),
        1,
    )
}

#[test]
fn shell_export_oracle_changes_only_the_app_info_version() {
    let actual = synthetic_current_export();
    assert!(actual.contains("\"nested\": {\"version\": \"0.0.0\"}"));
    assert_eq!(golden_with_package_version(SYNTHETIC_GOLDEN), actual);
    assert_shell_export_matches_golden(&actual, SYNTHETIC_GOLDEN);
}

#[test]
fn shell_export_oracle_rejects_wrong_emitted_version() {
    // Even a byte-identical old export must fail the package-version assertion.
    assert!(
        std::panic::catch_unwind(|| {
            assert_shell_export_matches_golden(SYNTHETIC_GOLDEN, SYNTHETIC_GOLDEN);
        })
        .is_err()
    );
}

#[test]
fn shell_export_oracle_rejects_body_byte_format_and_float_changes() {
    let actual = synthetic_current_export();
    for changed in [
        actual.replace("synthetic", "different"),
        actual.replace("  \"nested\":", " \"nested\":"),
        actual.replace("1.2500", "1.25"),
        actual.replace("{\"version\": \"0.0.0\"}", "{\"version\": \"9.9.9\"}"),
    ] {
        assert!(
            std::panic::catch_unwind(|| {
                assert_shell_export_matches_golden(&changed, SYNTHETIC_GOLDEN);
            })
            .is_err()
        );
    }
}

#[test]
fn shell_export_oracle_rejects_missing_or_ambiguous_header_version() {
    for golden in [
        SYNTHETIC_GOLDEN.replace("    \"version\": \"0.0.0\",\n", ""),
        SYNTHETIC_GOLDEN.replace(
            "    \"version\": \"0.0.0\",\n",
            "    \"version\": \"0.0.0\",\n    \"version\": \"0.0.0\",\n",
        ),
        SYNTHETIC_GOLDEN.replace(
            "    \"version\": \"0.0.0\",\n",
            "    \"version\": \"0.0.0\",\n    \"version\": \"9.9.9\",\n",
        ),
    ] {
        assert!(
            std::panic::catch_unwind(|| {
                golden_with_package_version(&golden);
            })
            .is_err()
        );
    }
}

#[test]
fn shell_export_oracle_rejects_unrecognized_header_format() {
    for golden in [
        SYNTHETIC_GOLDEN.replacen("  \"app_info\": {", " \"app_info\": {", 1),
        SYNTHETIC_GOLDEN.replacen("\n  },\n", "\n },\n", 1),
        SYNTHETIC_GOLDEN.replacen("    \"version\":", "    \"version\" :", 1),
    ] {
        assert!(
            std::panic::catch_unwind(|| {
                golden_with_package_version(&golden);
            })
            .is_err()
        );
    }
}

#[test]
fn generated_dto_export_matches_complete_committed_directory() {
    let root = tempfile::TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["dto-export", "--out"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let committed = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../apps/desktop/ui/src/data/generated");
    let files = |root: &std::path::Path| {
        fs::read_dir(root)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                assert!(entry.file_type().unwrap().is_file());
                (entry.file_name(), fs::read(entry.path()).unwrap())
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert_eq!(files(root.path()), files(&committed));
}

/// A populated fixture that links no pull request still exports: its
/// pull-request sections are empty and its report attempted nothing. Only the
/// user's refresh command requires a non-empty selection.
#[test]
fn a_fixture_without_pull_requests_exports_empty_pull_request_sections() {
    for id in ["F3", "F8"] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["fixture-export", id, "--shell"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{id}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let export: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            export["pull_requests"],
            serde_json::json!({ "rows": [] }),
            "{id}"
        );
        assert_eq!(
            export["pull_requests_refreshed"],
            serde_json::json!({ "rows": [] }),
            "{id}"
        );
        assert_eq!(
            export["pr_refresh"],
            serde_json::json!({
                "requested": 0,
                "attempted": 0,
                "succeeded": 0,
                "failed": 0,
                "skipped": 0,
                "unrecorded": 0,
                "cancelled": false,
                "committed": false,
                "rows": [],
            }),
            "{id}"
        );
    }
}
