#[test]
fn conformance_flush_turn() {
    let Some(plugin_root) = std::env::var_os("AGENT_PLUGINS_DIR") else {
        eprintln!("SKIP conformance_flush_turn: set AGENT_PLUGINS_DIR to the pinned plugin root");
        return;
    };
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    if std::process::Command::new(&python)
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("SKIP conformance_flush_turn: Python 3 is unavailable");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new(python)
        .arg(root.join("scripts/conformance/test-plugin-import.py"))
        .arg("--plugin-root")
        .arg(plugin_root)
        .args([
            "--expected-commit",
            "fae871a5c3385d14e55ea64f6bda7421c52ebebb",
            "--binary",
            env!("CARGO_BIN_EXE_xtrace-core"),
        ])
        .output()
        .expect("Run the pinned plugin conformance harness");
    print!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "Plugin conformance failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
