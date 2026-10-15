use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

fn alternate_catalog() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::TempDir::new().unwrap();
    let catalog = directory.path().canonicalize().unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/F1");
    for file in [
        "manifest.json",
        "expected.json",
        "input/prices.test.json",
        "input/hands-off.json",
        "input/sessions/session.jsonl",
    ] {
        let target = catalog.join("F1").join(file);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(source.join(file), target).unwrap();
    }
    (directory, catalog)
}

fn shell_export(catalog: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["fixture-export", "F1", "--shell", "--catalog"])
        .arg(catalog)
        .output()
        .unwrap()
}

#[test]
fn shell_export_accepts_an_alternate_catalog_with_only_the_selected_fixture() {
    let (_directory, catalog) = alternate_catalog();
    assert!(!catalog.join("F16").exists());
    let canonical = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["fixture-export", "F1", "--catalog"])
        .arg(&catalog)
        .output()
        .unwrap();
    assert!(
        canonical.status.success(),
        "{}",
        String::from_utf8_lossy(&canonical.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&canonical.stdout).unwrap();
    assert_eq!(value["manifest"]["id"], "F1");
    assert_eq!(
        value["sessions"][0]["records"].as_array().unwrap().len(),
        25
    );

    let alternate_shell = shell_export(&catalog);
    assert!(
        alternate_shell.status.success(),
        "{}",
        String::from_utf8_lossy(&alternate_shell.stderr)
    );
    // Complete byte parity also proves selected metadata, counts and pinned
    // clock survive, while Environment uses the bundled synthetic matrix.
    assert_eq!(
        std::str::from_utf8(&alternate_shell.stdout).unwrap(),
        include_str!("../../apps/desktop/ui/fixtures/F1.json")
    );
}

#[test]
fn shell_export_uses_a_supplied_environment_matrix_and_selected_fixture_clock() {
    let (_directory, catalog) = alternate_catalog();
    let base = catalog.join("F16/input/probes");
    fs::create_dir_all(base.join("env/home")).unwrap();
    fs::write(
        base.join("env.json"),
        r#"{"roots":{"home":"env/home","repositories":[]}}"#,
    )
    .unwrap();
    fs::write(
        base.join("env/home/.claude.json"),
        r#"{"mcpServers":{"alternate-catalog-server":{}}}"#,
    )
    .unwrap();
    let manifest = catalog.join("F1/manifest.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["now"] = serde_json::json!("2026-09-09T00:00:00Z");
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();

    let output = shell_export(&catalog);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["app_info"]["fixture"], "F1");
    assert_eq!(
        value["db_counts"],
        serde_json::json!({"sessions":1,"records":25,"usage":15})
    );
    let dashboards = value["dashboards"].as_array().unwrap();
    assert_eq!(dashboards.len(), 3);
    for dashboard in dashboards {
        assert_eq!(dashboard["window"]["clock"], "fixture");
        assert_eq!(dashboard["window"]["end_ms"], 1788912000000_i64);
    }
    let environments = value["environments"].as_array().unwrap();
    assert_eq!(environments.len(), 3);
    for environment in environments {
        assert_eq!(environment["window"]["clock"], "fixture");
        assert_eq!(environment["window"]["end_ms"], 1788912000000_i64);
        let configured = environment["configured"].as_array().unwrap();
        assert_eq!(configured.len(), 1);
        assert_eq!(configured[0]["name"], "alternate-catalog-server");
    }
}

#[test]
fn shell_export_rejects_present_invalid_environment_indexes_without_output() {
    for index in ["not JSON", r#"{"roots":{"home":42,"repositories":[]}}"#] {
        let (_directory, catalog) = alternate_catalog();
        let path = catalog.join("F16/input/probes/env.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, index).unwrap();
        assert_rejected_shell_output(&catalog);
    }
    let (_directory, catalog) = alternate_catalog();
    fs::create_dir_all(catalog.join("F16/input/probes/env.json")).unwrap();
    assert_rejected_shell_output(&catalog);

    #[cfg(unix)]
    {
        let (_directory, catalog) = alternate_catalog();
        let path = catalog.join("F16/input/probes/env.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("absent-index.json", path).unwrap();
        assert_rejected_shell_output(&catalog);
    }
}

#[test]
fn shell_export_rejects_empty_supplied_matrix_roots_without_output_or_fallback() {
    let mut mismatches = Vec::new();
    for invalid_home in [true, false] {
        let (_directory, catalog) = alternate_catalog();
        let base = catalog.join("F16/input/probes");
        fs::create_dir_all(base.join("env/home")).unwrap();
        for (file, name) in [
            (".claude.json", "index-parent-home-server"),
            (".mcp.json", "index-parent-repo-server"),
        ] {
            fs::write(
                base.join(file),
                serde_json::to_vec(&serde_json::json!({"mcpServers":{name:{}}})).unwrap(),
            )
            .unwrap();
        }
        let home = if invalid_home { "" } else { "env/home" };
        let repositories = if invalid_home { vec![] } else { vec![""] };
        fs::write(
            base.join("env.json"),
            serde_json::to_vec(
                &serde_json::json!({"roots":{"home":home,"repositories":repositories}}),
            )
            .unwrap(),
        )
        .unwrap();
        let output_path = catalog.join("export.json");
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["fixture-export", "F1", "--shell", "--catalog"])
            .arg(&catalog)
            .arg("--out")
            .arg(&output_path)
            .output()
            .unwrap();
        if output.status.success() {
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
            let sentinel = if invalid_home {
                "index-parent-home-server"
            } else {
                "index-parent-repo-server"
            };
            for environment in value["environments"].as_array().unwrap() {
                assert!(
                    environment["configured"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|component| component["name"] == sentinel)
                );
            }
            mismatches.push(format!(
                "empty {} succeeded, wrote output, and read {sentinel}",
                if invalid_home { "home" } else { "repository" }
            ));
        } else {
            assert_eq!(output.status.code(), Some(2));
            assert!(output.stdout.is_empty());
            assert!(!output.stderr.is_empty());
            assert!(!output_path.exists());
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

fn assert_rejected_shell_output(catalog: &std::path::Path) {
    let output_path = catalog.join("export.json");
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["fixture-export", "F1", "--shell", "--catalog"])
        .arg(catalog)
        .arg("--out")
        .arg(&output_path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
    assert!(!output_path.exists());
}

#[test]
fn generated_shell_export_uses_ipc_structs_and_rejects_skeletons() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["fixture-export", "F1", "--shell"])
        .output()
        .unwrap();
    assert!(output.status.success());
    // Byte parity: parsed floats are not round-trip exact under serde_json defaults.
    assert_eq!(
        std::str::from_utf8(&output.stdout).unwrap(),
        include_str!("../../apps/desktop/ui/fixtures/F1.json")
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
