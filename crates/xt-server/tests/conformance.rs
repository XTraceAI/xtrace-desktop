//! Pinned real-plugin conformance for the local server: the exact producer named
//! by `.plugin-pin` drives imports through its real Stop hook and speaks to the
//! headless binary with its real decoder and token client.
use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn pinned_commit() -> String {
    let text = std::fs::read_to_string(repo_root().join(".plugin-pin")).expect("read .plugin-pin");
    let pin: serde_json::Value = serde_json::from_str(&text).expect(".plugin-pin is JSON");
    pin["commit"]
        .as_str()
        .expect(".plugin-pin names a commit")
        .to_owned()
}

fn run_harness(name: &str, script: &str) {
    let Some(plugin_root) = std::env::var_os("AGENT_PLUGINS_DIR") else {
        println!("SKIP {name}: set AGENT_PLUGINS_DIR to the pinned plugin root");
        return;
    };
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    if Command::new(&python).arg("--version").output().is_err() {
        println!("SKIP {name}: Python 3 is unavailable");
        return;
    }
    let output = Command::new(python)
        .arg(repo_root().join(script))
        .arg("--plugin-root")
        .arg(plugin_root)
        .args(["--expected-commit", &pinned_commit()])
        .args(["--binary", env!("CARGO_BIN_EXE_xtrace-core")])
        .output()
        .expect("Run the pinned plugin conformance harness");
    print!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "Plugin conformance failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn conformance_flush_turn() {
    run_harness(
        "conformance_flush_turn",
        "scripts/conformance/test-plugin-import.py",
    );
}

#[test]
fn conformance_plugin_transport() {
    run_harness(
        "conformance_plugin_transport",
        "scripts/conformance/test-plugin-transport.py",
    );
}
