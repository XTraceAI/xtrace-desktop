//! The headless `import-native` command: a JSON report on stdout, exit 0 only
//! when every requested host imported completely, 2 for an explicit gap.
use serde_json::Value;
use std::{fs, io::Write, process::Command};

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

    // A relative --home is anchored to the invoking directory; reported paths
    // are absolute and the repeated import adds nothing.
    let output = core()
        .current_dir(temp.path())
        .args(["import-native", "--db"])
        .arg(&db)
        .args(["--home", "./home", "--host", "claude"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["hosts"][0]["status"], "complete");
    assert_eq!(report["hosts"][0]["sessions"][0]["records_new"], 0);
    let path = report["hosts"][0]["sessions"][0]["path"].as_str().unwrap();
    assert!(std::path::Path::new(path).is_absolute(), "{path}");
    assert!(!path.contains("/./"), "{path}");

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

#[cfg(unix)]
#[test]
fn database_and_sidecar_aliases_cannot_modify_native_history() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let sources = home.join(".cursor/chats/session");
    fs::create_dir_all(&sources).unwrap();
    let source = sources.join("store.db");
    fs::write(&source, "").unwrap();
    let link = temp.path().join("linked.sqlite");
    symlink(&source, &link).unwrap();
    let hard = temp.path().join("hard.sqlite");
    fs::hard_link(&source, &hard).unwrap();
    let sidecar_db = temp.path().join("sidecar.sqlite");
    symlink(&source, temp.path().join("sidecar.sqlite-wal")).unwrap();
    for db in [
        &source,
        &link,
        &hard,
        &sidecar_db,
        &sources.join("new.sqlite"),
    ] {
        let output = core()
            .args(["import-native", "--db"])
            .arg(db)
            .arg("--home")
            .arg(&home)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(fs::read(&source).unwrap(), b"");
    }
    assert!(!sidecar_db.exists());
    assert!(!sources.join("new.sqlite").exists());
    assert_eq!(fs::read_dir(&sources).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn native_reverse_symlinks_cannot_expose_the_index_as_source_history() {
    use std::os::unix::fs::symlink;
    for directory in [false, true] {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("home");
        let source = home.join(".cursor/chats/session");
        fs::create_dir_all(&source).unwrap();
        let data = temp.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let db = data.join("store.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("CREATE TABLE native_data(value TEXT)", [])
            .unwrap();
        drop(conn);
        let original = fs::read(&db).unwrap();
        if directory {
            symlink(&data, source.join("linked")).unwrap();
        } else {
            symlink(&db, source.join("store.db")).unwrap();
        }
        let output = core()
            .args(["import-native", "--db"])
            .arg(&db)
            .arg("--home")
            .arg(&home)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(fs::read(&db).unwrap(), original);
        assert_eq!(fs::read_dir(&data).unwrap().count(), 1);
    }
}

/// `watch-native --once`: the watcher registers, the initial scan runs, queued
/// changes are reconciled, one `ready` line is printed, and the process exits
/// by freshness and completeness. A second run resumes behind checkpoints.
#[test]
fn watch_native_once_reports_ready_and_resumes_on_the_next_run() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-synthetic");
    fs::create_dir_all(&project).unwrap();
    let session = "00000000-0000-4000-8000-00000000c1b0";
    let record = |index: u32| {
        format!(
            "{{\"uuid\":\"44444444-4444-4444-8444-{index:012}\",\"type\":\"user\",\"sessionId\":\"{session}\",\"entrypoint\":\"cli\",\"timestamp\":\"2026-09-07T12:00:0{index}Z\",\"message\":{{\"role\":\"user\",\"content\":\"turn {index}\"}}}}\n"
        )
    };
    fs::write(
        project.join(format!("{session}.jsonl")),
        record(1) + &record(2),
    )
    .unwrap();
    let db = temp.path().join("index.sqlite");
    let run = || {
        core()
            .args(["watch-native", "--db"])
            .arg(&db)
            .arg("--home")
            .arg(&home)
            .args(["--host", "claude", "--once"])
            .output()
            .unwrap()
    };
    let output = run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["event"], "ready");
    assert_eq!(lines[0]["freshness"], "live");
    assert_eq!(lines[0]["report"]["hosts"][0]["status"], "complete");
    assert_eq!(
        lines[0]["report"]["hosts"][0]["sessions"][0]["records_new"],
        2
    );
    assert_eq!(lines.last().unwrap()["event"], "stopped");
    assert_eq!(lines.last().unwrap()["freshness"], "live");
    // Appended offline; the next run reads only the new record.
    fs::OpenOptions::new()
        .append(true)
        .open(project.join(format!("{session}.jsonl")))
        .unwrap()
        .write_all(record(3).as_bytes())
        .unwrap();
    let output = run();
    assert!(output.status.success());
    let ready: Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(ready["report"]["hosts"][0]["sessions"][0]["records_new"], 1);
    assert_eq!(
        ready["report"]["hosts"][0]["sessions"][0]["source_surface"],
        "cli"
    );
    // `--replay` on the one-shot import rereads everything and adds nothing.
    let output = core()
        .args(["import-native", "--db"])
        .arg(&db)
        .arg("--home")
        .arg(&home)
        .args(["--host", "claude", "--replay"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["hosts"][0]["sessions"][0]["records_new"], 0);
    assert_eq!(report["hosts"][0]["sessions"][0]["source_surface"], "cli");
    // Usage errors stay explicit.
    for args in [
        vec![
            "watch-native",
            "--db",
            "x",
            "--home",
            ".",
            "--once",
            "--once",
        ],
        vec!["watch-native", "--db", "x", "--home", ".", "--for", "soon"],
        vec!["import-native", "--db", "x", "--home", ".", "--once"],
    ] {
        let output = core().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
    }
}
