use std::{fs, process::Command};

#[test]
fn generated_shell_export_uses_ipc_structs_and_rejects_skeletons() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["fixture-export", "F1", "--shell"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("../../apps/desktop/ui/fixtures/F1.json")).unwrap();
    assert_eq!(actual, expected);
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
