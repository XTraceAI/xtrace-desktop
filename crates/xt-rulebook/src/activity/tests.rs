//! Synthetic acceptance for the rule activity reader. Every source lives in a
//! temporary directory; no real home, ledger or plugin file is touched.

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use jiff::Timestamp;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::*;

const SECRETS: [&str; 6] = [
    "SECRET-EXCERPT-7f3a",
    "SECRET-OVERRIDE-91c2",
    "SECRET-MESSAGE-44d0",
    "SECRET-DEDUP-0b7e",
    "SECRET-UNKNOWN-5e11",
    "SECRET-NESTED-c9a8",
];

/// A synthetic home with the default root under it.
struct Source {
    _home: TempDir,
    root: PathBuf,
}

impl Source {
    /// Root and `ledger/` exist; nothing else.
    fn bare() -> Self {
        let home = tempfile::tempdir().unwrap();
        let root = default_root(home.path());
        fs::create_dir_all(root.join("ledger")).unwrap();
        Self { _home: home, root }
    }

    /// A schema-2 source with `ledger` as the fires file.
    fn with(ledger: &str) -> Self {
        let source = Self::bare();
        source.marker(b"2\n");
        source.ledger(ledger.as_bytes());
        source
    }

    fn marker(&self, bytes: &[u8]) {
        fs::write(self.root.join("ledger/schema_version"), bytes).unwrap();
    }

    fn ledger(&self, bytes: &[u8]) {
        fs::write(self.ledger_path(), bytes).unwrap();
    }

    fn append(&self, bytes: &[u8]) {
        use std::io::Write;
        fs::OpenOptions::new()
            .append(true)
            .open(self.ledger_path())
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }

    fn ledger_path(&self) -> PathBuf {
        self.root.join("ledger/fires.jsonl")
    }

    fn read(&self) -> ActivityRead {
        self.read_with(&ReadLimits::default(), None)
    }

    fn read_with(&self, limits: &ReadLimits, window: Option<ActivityWindow>) -> ActivityRead {
        read_activity(&self.root, limits, window, &|| false)
    }

    fn snapshot(&self) -> ActivitySnapshot {
        expect_snapshot(self.read())
    }
}

fn expect_snapshot(read: ActivityRead) -> ActivitySnapshot {
    match read {
        ActivityRead::Snapshot(snapshot) => *snapshot,
        other => panic!("expected a snapshot, got {other:?}"),
    }
}

fn path_problem(read: ActivityRead) -> (SourcePart, PathProblem) {
    match read {
        ActivityRead::Unavailable(Unavailable::Path { part, problem }) => (part, problem),
        other => panic!("expected a path problem, got {other:?}"),
    }
}

fn ts(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// A complete schema-2 row, as the plugin writes it, carrying excluded
/// content fields full of secrets.
fn row_value(fire_id: &str, fired_at: &str, mode: &str) -> Value {
    json!({
        "fire_id": fire_id,
        "rule_id": "rule-no-force-push",
        "rulebook_id": "book-1",
        "rule_version": 3,
        "session_id": "0d9c7a52-native",
        "agent_id": null,
        "worktree": "1a2b3c4d5e6f7a8b",
        "host": "claude",
        "source_message_id": SECRETS[2],
        "repo": "acme/widgets",
        "branch": "main",
        "tool": "Bash",
        "hook_phase": "pre",
        "mode": mode,
        "dedup_key": SECRETS[3],
        "raw_matches_before_fire": 2,
        "fired_at": fired_at,
        "override_reason": SECRETS[1],
        "excerpt": SECRETS[0],
        "future_field": SECRETS[4],
        "nested": {"deep": [SECRETS[5]]},
    })
}

fn line(value: &Value) -> String {
    format!("{}\n", serde_json::to_string(value).unwrap())
}

fn row(fire_id: &str, fired_at: &str, mode: &str) -> String {
    line(&row_value(fire_id, fired_at, mode))
}

fn with_field(fire_id: &str, key: &str, value: Value) -> String {
    let mut row = row_value(fire_id, "2026-09-20T10:00:00.000001+00:00", "advise");
    row[key] = value;
    line(&row)
}

fn without_field(fire_id: &str, key: &str) -> String {
    let mut row = row_value(fire_id, "2026-09-20T10:00:00.000001+00:00", "advise");
    row.as_object_mut().unwrap().remove(key);
    line(&row)
}

/// `count` rows one minute apart, fire IDs `f0000`…, oldest first.
fn rows(count: usize) -> String {
    let base = ts("2026-09-01T00:00:00Z");
    (0..count)
        .map(|i| {
            let at = base + jiff::SignedDuration::from_secs(60 * i as i64);
            row(&format!("f{i:05}"), &at.to_string(), "advise")
        })
        .collect()
}

/// Like [`rows`] but with required fields only, so many fit in the tail.
fn compact_rows(count: usize) -> String {
    let base = ts("2026-09-01T00:00:00Z");
    (0..count)
        .map(|i| {
            let at = base + jiff::SignedDuration::from_secs(60 * i as i64);
            line(&json!({
                "fire_id": format!("f{i:05}"),
                "rule_id": "r",
                "session_id": "s",
                "mode": "advise",
                "fired_at": at.to_string(),
            }))
        })
        .collect()
}

fn assert_no_secrets(read: &ActivityRead) {
    let debug = format!("{read:?}");
    for secret in SECRETS {
        assert!(!debug.contains(secret), "{secret} crossed into the result");
    }
}

/// Every path under `dir` with its bytes and modification time.
fn tree(dir: &Path) -> Vec<(PathBuf, Option<Vec<u8>>, SystemTime)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let meta = fs::symlink_metadata(&path).unwrap();
        let bytes = meta.is_file().then(|| fs::read(&path).unwrap());
        out.push((path.clone(), bytes, meta.modified().unwrap()));
        if meta.is_dir() {
            for entry in fs::read_dir(&path).unwrap() {
                stack.push(entry.unwrap().path());
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// ── root, paths and schema ─────────────────────────────────────────────────

#[test]
fn default_root_is_under_the_supplied_home() {
    assert_eq!(
        default_root(Path::new("/synthetic/home")),
        Path::new("/synthetic/home/.config/memhub-plugin/rulebook")
    );
}

#[test]
fn a_relative_root_is_refused() {
    let read = read_activity(
        Path::new("relative/rulebook"),
        &ReadLimits::default(),
        None,
        &|| false,
    );
    assert_eq!(
        read,
        ActivityRead::Unavailable(Unavailable::RootNotAbsolute)
    );
}

#[test]
fn missing_paths_are_unavailable_and_nothing_is_created() {
    let home = tempfile::tempdir().unwrap();
    let root = default_root(home.path());
    let read = read_activity(&root, &ReadLimits::default(), None, &|| false);
    assert_eq!(path_problem(read), (SourcePart::Root, PathProblem::Missing));
    assert!(!root.exists());

    fs::create_dir_all(&root).unwrap();
    let read = read_activity(&root, &ReadLimits::default(), None, &|| false);
    assert_eq!(
        path_problem(read),
        (SourcePart::LedgerDir, PathProblem::Missing)
    );
    assert!(!root.join("ledger").exists());

    // Unlike the plugin's own ledger helper, the reader never writes the
    // marker it finds missing.
    let source = Source::bare();
    source.ledger(row("a", "2026-09-20T10:00:00Z", "gate").as_bytes());
    assert_eq!(
        path_problem(source.read()),
        (SourcePart::SchemaMarker, PathProblem::Missing)
    );
    assert!(!source.root.join("ledger/schema_version").exists());

    let source = Source::bare();
    source.marker(b"2\n");
    assert_eq!(
        path_problem(source.read()),
        (SourcePart::Ledger, PathProblem::Missing)
    );
    assert!(!source.ledger_path().exists());
}

#[test]
fn a_root_that_is_a_file_is_not_a_directory() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("rulebook");
    fs::write(&root, b"").unwrap();
    let read = read_activity(&root, &ReadLimits::default(), None, &|| false);
    assert_eq!(
        path_problem(read),
        (SourcePart::Root, PathProblem::NotDirectory)
    );
}

#[test]
fn the_schema_marker_must_be_exactly_two() {
    for accepted in [&b"2\n"[..], b"2"] {
        let source = Source::with("");
        source.marker(accepted);
        assert_eq!(source.snapshot().schema_version, 2);
    }
    for (bytes, found) in [(&b"3\n"[..], 3), (b"1\n", 1), (b"10", 10)] {
        let source = Source::with("");
        source.marker(bytes);
        assert_eq!(
            source.read(),
            ActivityRead::Unavailable(Unavailable::SchemaUnsupported { found })
        );
    }
    for malformed in [
        &b""[..],
        b"\n",
        b" 2\n",
        b"2 \n",
        b"2\n\n",
        b"2\r\n",
        b"02\n",
        b"+2\n",
        b"2.0\n",
        b"two\n",
        b"{\"schema\":2}\n",
        b"99999999999999999999999\n",
        b"\xff\n",
    ] {
        let source = Source::with("");
        source.marker(malformed);
        assert_eq!(
            source.read(),
            ActivityRead::Unavailable(Unavailable::SchemaMalformed),
            "marker {malformed:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn links_and_non_regular_paths_are_refused() {
    use std::os::unix::fs::symlink;

    // The fires file is a link to a valid ledger inside the root.
    let source = Source::with("");
    let real = source.root.join("ledger/real.jsonl");
    fs::write(&real, row("a", "2026-09-20T10:00:00Z", "gate")).unwrap();
    fs::remove_file(source.ledger_path()).unwrap();
    symlink(&real, source.ledger_path()).unwrap();
    assert_eq!(
        path_problem(source.read()),
        (SourcePart::Ledger, PathProblem::Symlink)
    );

    // The marker is a link to a valid marker.
    let source = Source::with("");
    let real = source.root.join("real_marker");
    fs::write(&real, b"2\n").unwrap();
    fs::remove_file(source.root.join("ledger/schema_version")).unwrap();
    symlink(&real, source.root.join("ledger/schema_version")).unwrap();
    assert_eq!(
        path_problem(source.read()),
        (SourcePart::SchemaMarker, PathProblem::Symlink)
    );

    // The ledger directory is a link to a valid ledger directory elsewhere.
    let source = Source::with(&row("a", "2026-09-20T10:00:00Z", "gate"));
    let elsewhere = tempfile::tempdir().unwrap();
    let moved = elsewhere.path().join("ledger");
    fs::rename(source.root.join("ledger"), &moved).unwrap();
    symlink(&moved, source.root.join("ledger")).unwrap();
    assert_eq!(
        path_problem(source.read()),
        (SourcePart::LedgerDir, PathProblem::Symlink)
    );

    // A directory where the fires file should be.
    let source = Source::with("");
    fs::remove_file(source.ledger_path()).unwrap();
    fs::create_dir(source.ledger_path()).unwrap();
    assert_eq!(
        path_problem(source.read()),
        (SourcePart::Ledger, PathProblem::NotRegularFile)
    );

    // A file where the ledger directory should be.
    let source = Source::bare();
    fs::remove_dir(source.root.join("ledger")).unwrap();
    fs::write(source.root.join("ledger"), b"").unwrap();
    assert_eq!(
        path_problem(source.read()),
        (SourcePart::LedgerDir, PathProblem::NotDirectory)
    );

    // A FIFO is refused without blocking.
    let source = Source::with("");
    fs::remove_file(source.ledger_path()).unwrap();
    let fifo = std::ffi::CString::new(source.ledger_path().to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path; mkfifo only creates the node.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert_eq!(
        path_problem(source.read()),
        (SourcePart::Ledger, PathProblem::NotRegularFile)
    );

    // A hard link may share the file with a location outside the root.
    let source = Source::with(&row("a", "2026-09-20T10:00:00Z", "gate"));
    let outside = tempfile::tempdir().unwrap();
    fs::hard_link(source.ledger_path(), outside.path().join("shared")).unwrap();
    assert_eq!(
        path_problem(source.read()),
        (SourcePart::Ledger, PathProblem::MultipleLinks)
    );
}

#[cfg(unix)]
#[test]
fn a_linked_root_is_followed_and_reported_canonically() {
    use std::os::unix::fs::symlink;
    let source = Source::with(&row("a", "2026-09-20T10:00:00Z", "gate"));
    let other = tempfile::tempdir().unwrap();
    let link = other.path().join("linked-root");
    symlink(&source.root, &link).unwrap();
    let snapshot = expect_snapshot(read_activity(&link, &ReadLimits::default(), None, &|| {
        false
    }));
    assert_eq!(snapshot.root, fs::canonicalize(&source.root).unwrap());
    assert_eq!(snapshot.counts.distinct, 1);
}

#[cfg(unix)]
#[test]
fn an_unreadable_ledger_is_unavailable() {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        return; // root ignores file modes
    }
    let source = Source::with(&row("a", "2026-09-20T10:00:00Z", "gate"));
    fs::set_permissions(source.ledger_path(), fs::Permissions::from_mode(0o000)).unwrap();
    let read = source.read();
    fs::set_permissions(source.ledger_path(), fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        path_problem(read),
        (
            SourcePart::Ledger,
            PathProblem::Unreadable(std::io::ErrorKind::PermissionDenied)
        )
    );
}

// ── rows, modes and privacy ────────────────────────────────────────────────

#[test]
fn an_empty_ledger_is_exactly_zero_recorded_rows() {
    let snapshot = Source::with("").snapshot();
    assert_eq!(snapshot.counts.precision, Precision::Exact);
    assert_eq!(snapshot.counts.lines, 0);
    assert_eq!(snapshot.counts.distinct, 0);
    assert_eq!(snapshot.counts.in_window, ModeCounts::default());
    assert!(snapshot.rows.is_empty());
    assert!(snapshot.groups.is_empty());
    assert_eq!(snapshot.counts.observed_groups, 0);
    assert_eq!(snapshot.coverage.captured_len, 0);
    assert_eq!(snapshot.coverage.scanned, 0..0);
    assert_eq!(snapshot.coverage.oldest_observed, None);
}

#[test]
fn rows_carry_only_whitelisted_typed_metadata() {
    let ledger = row("a", "2026-09-20T10:00:00.000001+02:00", "gate");
    let source = Source::with(&ledger);
    let read = source.read();
    assert_no_secrets(&read);
    let snapshot = expect_snapshot(read);
    assert_eq!(snapshot.counts.precision, Precision::Exact);
    assert_eq!(
        snapshot.rows,
        vec![FireRecord {
            fire_id: "a".into(),
            rule_id: "rule-no-force-push".into(),
            rule_version: Some(RuleVersion::Number(3)),
            rulebook_id: Some("book-1".into()),
            session_id: "0d9c7a52-native".into(),
            agent_id: None,
            worktree: Some("1a2b3c4d5e6f7a8b".into()),
            host: Some("claude".into()),
            repo: Some("acme/widgets".into()),
            branch: Some("main".into()),
            tool: Some("Bash".into()),
            hook_phase: Some("pre".into()),
            mode: FireMode::Gate,
            fired_at: ts("2026-09-20T08:00:00.000001Z"),
        }]
    );
    assert_eq!(snapshot.coverage.captured_len, ledger.len() as u64);
    assert_eq!(snapshot.coverage.scanned, 0..ledger.len() as u64);
}

#[test]
fn modes_are_recorded_modes_and_unknown_modes_stay_visible() {
    let ledger = [
        row("a", "2026-09-20T10:00:01Z", "advise"),
        row("b", "2026-09-20T10:00:02Z", "gate"),
        row("c", "2026-09-20T10:00:03Z", "suppressed"),
        row("d", "2026-09-20T10:00:04Z", "shadow"),
        row("e", "2026-09-20T10:00:05Z", "Gate"),
        row("f", "2026-09-20T10:00:06Z", "blocked"),
    ]
    .concat();
    let snapshot = Source::with(&ledger).snapshot();
    let modes = snapshot.counts.in_window;
    assert_eq!(
        modes,
        ModeCounts {
            advise: 1,
            gate: 1,
            suppressed: 1,
            unrecognized: 3,
        }
    );
    assert_eq!(modes.recorded_fires(), 2);
    assert_eq!(modes.total(), 6);
    let recorded: Vec<_> = snapshot.rows.iter().map(|r| r.mode.clone()).collect();
    assert_eq!(
        recorded,
        vec![
            FireMode::Unrecognized("blocked".into()),
            FireMode::Unrecognized("Gate".into()),
            FireMode::Unrecognized("shadow".into()),
            FireMode::Suppressed,
            FireMode::Gate,
            FireMode::Advise,
        ]
    );
}

#[test]
fn raw_host_and_session_identities_are_not_normalized() {
    let ledger = [
        with_field("a", "session_id", json!("codex-019a")),
        with_field("b", "session_id", json!("019a")),
        with_field("c", "host", json!("codex")),
        with_field("d", "host", json!("some-future-host")),
        without_field("e", "host"),
        with_field("f", "session_id", json!(" padded ")),
    ]
    .concat();
    let snapshot = Source::with(&ledger).snapshot();
    let by_id = |id: &str| snapshot.rows.iter().find(|r| r.fire_id == id).unwrap();
    assert_eq!(by_id("a").session_id, "codex-019a");
    assert_eq!(by_id("b").session_id, "019a");
    assert_eq!(by_id("c").host.as_deref(), Some("codex"));
    assert_eq!(by_id("d").host.as_deref(), Some("some-future-host"));
    assert_eq!(by_id("e").host, None);
    assert_eq!(by_id("f").session_id, " padded ");
}

#[test]
fn optional_fields_may_be_absent_or_null_and_versions_are_typed() {
    let optional = [
        "rulebook_id",
        "agent_id",
        "worktree",
        "host",
        "repo",
        "branch",
        "tool",
        "hook_phase",
        "rule_version",
    ];
    let mut ledger = String::new();
    for (i, key) in optional.iter().enumerate() {
        ledger += &without_field(&format!("absent-{i}"), key);
        ledger += &with_field(&format!("null-{i}"), key, Value::Null);
    }
    ledger += &with_field("label", "rule_version", json!("v7-beta"));
    ledger += &with_field("empty-branch", "branch", json!(""));
    let snapshot = Source::with(&ledger).snapshot();
    assert_eq!(snapshot.counts.malformed.total(), 0);
    assert_eq!(snapshot.counts.distinct, optional.len() as u64 * 2 + 2);
    let by_id = |id: &str| snapshot.rows.iter().find(|r| r.fire_id == id).unwrap();
    assert_eq!(by_id("absent-8").rule_version, None);
    assert_eq!(by_id("null-8").rule_version, None);
    assert_eq!(by_id("absent-0").rulebook_id, None);
    assert_eq!(by_id("null-3").host, None);
    assert_eq!(
        by_id("label").rule_version,
        Some(RuleVersion::Label("v7-beta".into()))
    );
    assert_eq!(by_id("empty-branch").branch.as_deref(), Some(""));
}

#[test]
fn malformed_rows_are_counted_by_reason_without_their_content() {
    let secret_row = |key: &str, value: Value| {
        let mut row = row_value("bad", "2026-09-20T10:00:00Z", "advise");
        row[key] = value;
        line(&row)
    };
    let invalid_json = [
        format!("{{\"fire_id\": \"{}\"\n", SECRETS[0]),
        format!(
            "{}garbage\n",
            row("x", "2026-09-20T10:00:00Z", "gate").trim_end()
        ),
        "{\"fire_id\": \"\\ud800\"}\n".to_owned(),
    ];
    let mut invalid_utf8 = row("y", "2026-09-20T10:00:00Z", "gate").into_bytes();
    let at = invalid_utf8.len() - 30;
    invalid_utf8[at] = 0xff;
    let invalid_shape = [
        format!("[\"{}\"]\n", SECRETS[0]),
        "\"a string\"\n".to_owned(),
        "42\n".to_owned(),
        without_field("s1", "fire_id"),
        without_field("s2", "rule_id"),
        without_field("s3", "session_id"),
        without_field("s4", "mode"),
        with_field("s5", "fire_id", json!("")),
        with_field("s6", "fire_id", json!("   ")),
        with_field("s7", "session_id", json!(12)),
        with_field("s8", "rule_id", json!(["rule"])),
        with_field("s9", "rule_version", json!(1.5)),
        with_field("s10", "rule_version", json!(true)),
        with_field("s11", "rule_version", json!(9_223_372_036_854_775_808_u64)),
        with_field("s12", "host", json!({"name": SECRETS[5]})),
        with_field("s13", "mode", json!("")),
        with_field("s14", "fire_id", Value::Null),
        secret_row("repo", json!(7)),
        // A repeated whitelisted key makes the row ambiguous.
        format!(
            "{{\"fire_id\":\"s15\",\"fire_id\":\"s16\",{}\n",
            &row("s15", "2026-09-20T10:00:00Z", "gate")[1..].trim_end()
        ),
    ];
    let invalid_timestamp = [
        secret_row("fired_at", json!("2026-09-20T10:00:00")),
        secret_row("fired_at", json!("yesterday")),
        secret_row("fired_at", json!("")),
        with_field("t1", "fired_at", json!(1_758_000_000)),
        without_field("t2", "fired_at"),
    ];
    let mut ledger = invalid_json.concat().into_bytes();
    ledger.extend_from_slice(&invalid_utf8);
    ledger.extend_from_slice(invalid_shape.concat().as_bytes());
    ledger.extend_from_slice(invalid_timestamp.concat().as_bytes());
    ledger.extend_from_slice(row("good", "2026-09-20T10:00:00Z", "gate").as_bytes());
    let source = Source::bare();
    source.marker(b"2\n");
    source.ledger(&ledger);

    let read = source.read();
    assert_no_secrets(&read);
    let snapshot = expect_snapshot(read);
    let malformed = snapshot.counts.malformed;
    assert_eq!(malformed.invalid_json, invalid_json.len() as u64 + 1);
    // Blank, numeric and missing timestamps are shape errors, not timestamps.
    assert_eq!(malformed.invalid_shape, invalid_shape.len() as u64 + 3);
    assert_eq!(
        malformed.invalid_timestamp,
        invalid_timestamp.len() as u64 - 3
    );
    assert_eq!(malformed.oversize_line + malformed.oversize_value, 0);
    assert_eq!(snapshot.counts.valid_rows, 1);
    assert_eq!(snapshot.rows.len(), 1);
    assert_eq!(snapshot.rows[0].fire_id, "good");
    assert_eq!(snapshot.counts.precision, Precision::LowerBound);
}

#[test]
fn oversize_values_are_rejected_never_truncated() {
    let at_limit = "r".repeat(ReadLimits::MAX_VALUE_BYTES);
    let over = "r".repeat(ReadLimits::MAX_VALUE_BYTES + 1);
    // Multi-byte characters count by UTF-8 bytes: 256 × 2 + 1 = 513 bytes.
    let over_multibyte = "é".repeat(256) + "x";
    let ledger = [
        with_field("fits", "rule_id", json!(at_limit)),
        with_field("long-rule", "rule_id", json!(over)),
        with_field("long-host", "host", json!(over)),
        with_field("long-version", "rule_version", json!(over)),
        with_field("long-mode", "mode", json!(over)),
        with_field("long-time", "fired_at", json!(over)),
        with_field("long-multibyte", "session_id", json!(over_multibyte)),
        with_field(&over, "rule_id", json!("r")),
    ]
    .concat();
    let snapshot = Source::with(&ledger).snapshot();
    assert_eq!(snapshot.counts.malformed.oversize_value, 7);
    assert_eq!(snapshot.rows.len(), 1);
    assert_eq!(snapshot.rows[0].rule_id, at_limit);
    assert_eq!(snapshot.counts.precision, Precision::LowerBound);

    // A tightened value bound applies the same way.
    let limits = ReadLimits::default().tighten_value_bytes(4);
    let read =
        Source::with(&with_field("abcd", "rule_id", json!("abcde"))).read_with(&limits, None);
    assert_eq!(expect_snapshot(read).counts.malformed.oversize_value, 1);
}

#[test]
fn oversize_lines_are_rejected_unparsed() {
    let padding = "p".repeat(ReadLimits::MAX_LINE_BYTES);
    let long = with_field("long", "padding", json!(padding));
    let short = row("short", "2026-09-20T10:00:00Z", "gate");
    let snapshot = Source::with(&(long + &short)).snapshot();
    assert_eq!(snapshot.counts.malformed.oversize_line, 1);
    assert_eq!(snapshot.counts.lines, 2);
    assert_eq!(snapshot.rows.len(), 1);

    // A line of exactly the bound is parsed.
    let base = with_field("exact", "padding", json!(""));
    let fill = ReadLimits::MAX_LINE_BYTES - (base.len() - 1);
    let exact = with_field("exact", "padding", json!("p".repeat(fill)));
    assert_eq!(exact.len() - 1, ReadLimits::MAX_LINE_BYTES);
    let snapshot = Source::with(&exact).snapshot();
    assert_eq!(snapshot.counts.malformed.total(), 0);
    assert_eq!(snapshot.rows.len(), 1);
}

#[test]
fn blank_lines_are_neither_records_nor_malformed() {
    let ledger = format!("\n   \n{}\t\n", row("a", "2026-09-20T10:00:00Z", "gate"));
    let snapshot = Source::with(&ledger).snapshot();
    assert_eq!(snapshot.counts.blank_lines, 3);
    assert_eq!(snapshot.counts.lines, 4);
    assert_eq!(snapshot.counts.malformed.total(), 0);
    assert_eq!(snapshot.counts.precision, Precision::Exact);
}

// ── duplicates and conflicts ───────────────────────────────────────────────

#[test]
fn identical_duplicates_count_once_and_keep_precision() {
    let a = row("a", "2026-09-20T10:00:00Z", "gate");
    // The same instant written with another offset is the same metadata;
    // excluded fields differing does not matter either.
    let mut same = row_value("a", "2026-09-20T12:00:00+02:00", "gate");
    same["excerpt"] = json!("another excerpt");
    let ledger = [
        a.clone(),
        a,
        line(&same),
        row("b", "2026-09-20T11:00:00Z", "advise"),
    ]
    .concat();
    let snapshot = Source::with(&ledger).snapshot();
    assert_eq!(snapshot.counts.valid_rows, 4);
    assert_eq!(snapshot.counts.distinct, 2);
    assert_eq!(snapshot.counts.duplicate_rows, 2);
    assert_eq!(snapshot.counts.conflicted_ids, 0);
    assert_eq!(snapshot.counts.in_window.total(), 2);
    assert_eq!(snapshot.rows.len(), 2);
    assert_eq!(snapshot.counts.precision, Precision::Exact);
}

#[test]
fn conflicting_metadata_for_one_fire_id_is_excluded_and_lowers_precision() {
    let ledger = [
        row("a", "2026-09-20T10:00:00Z", "gate"),
        row("a", "2026-09-20T10:00:00Z", "gate"),
        row("a", "2026-09-20T10:00:00Z", "advise"),
        with_field("b", "host", json!("claude")),
        with_field("b", "host", json!("codex")),
        with_field("c", "rule_version", json!(3)),
        with_field("c", "rule_version", json!("3")),
        row("d", "2026-09-20T10:00:00Z", "gate"),
        row("d", "2026-09-20T10:00:01Z", "gate"),
        row("ok", "2026-09-20T09:00:00Z", "advise"),
    ]
    .concat();
    let snapshot = Source::with(&ledger).snapshot();
    assert_eq!(snapshot.counts.conflicted_ids, 4);
    assert_eq!(snapshot.counts.conflicted_rows, 9);
    assert_eq!(snapshot.counts.distinct, 1);
    assert_eq!(snapshot.counts.duplicate_rows, 0);
    assert_eq!(
        snapshot.counts.in_window,
        ModeCounts {
            advise: 1,
            ..ModeCounts::default()
        }
    );
    assert_eq!(snapshot.rows.len(), 1);
    assert_eq!(snapshot.rows[0].fire_id, "ok");
    assert_eq!(
        snapshot.coverage.oldest_observed,
        Some(ts("2026-09-20T09:00:00Z"))
    );
    assert_eq!(
        snapshot.coverage.newest_observed,
        Some(ts("2026-09-20T09:00:00Z"))
    );
    assert_eq!(snapshot.counts.precision, Precision::LowerBound);
}

// ── ordering, window and presentation bound ────────────────────────────────

#[test]
fn rows_are_newest_first_with_fire_id_tie_break() {
    let ledger = [
        row("m", "2026-09-20T10:00:00Z", "gate"),
        row("z", "2026-09-20T09:00:00Z", "gate"),
        row("a", "2026-09-20T10:00:00Z", "gate"),
        row("q", "2026-09-20T12:00:00+03:00", "gate"),
        row("n", "2026-09-20T11:00:00Z", "gate"),
    ]
    .concat();
    let snapshot = Source::with(&ledger).snapshot();
    let order: Vec<_> = snapshot.rows.iter().map(|r| r.fire_id.as_str()).collect();
    // q is 09:00Z, tying z; ties order by descending fire_id.
    assert_eq!(order, ["n", "m", "a", "z", "q"]);
}

#[test]
fn the_window_is_half_open_and_applies_after_dedupe() {
    let ledger = [
        row("before", "2026-09-19T23:59:59.999999Z", "gate"),
        row("start", "2026-09-20T00:00:00Z", "gate"),
        row("start", "2026-09-20T00:00:00Z", "gate"),
        row("inside", "2026-09-21T00:00:00Z", "suppressed"),
        row("end", "2026-09-22T00:00:00Z", "advise"),
        row("conflict", "2026-09-21T00:00:00Z", "gate"),
        row("conflict", "2026-09-21T00:00:00Z", "advise"),
    ]
    .concat();
    let window = ActivityWindow::new(ts("2026-09-20T00:00:00Z"), ts("2026-09-22T00:00:00Z"));
    let read = Source::with(&ledger).read_with(&ReadLimits::default(), window);
    let snapshot = expect_snapshot(read);
    assert_eq!(snapshot.window, window);
    let ids: Vec<_> = snapshot.rows.iter().map(|r| r.fire_id.as_str()).collect();
    assert_eq!(ids, ["inside", "start"]);
    assert_eq!(
        snapshot.counts.in_window,
        ModeCounts {
            gate: 1,
            suppressed: 1,
            ..ModeCounts::default()
        }
    );
    // Snapshot-wide facts are not windowed.
    assert_eq!(snapshot.counts.distinct, 4);
    assert_eq!(snapshot.counts.duplicate_rows, 1);
    assert_eq!(snapshot.counts.conflicted_ids, 1);
    assert_eq!(
        snapshot.coverage.oldest_observed,
        Some(ts("2026-09-19T23:59:59.999999Z"))
    );
    assert_eq!(
        snapshot.coverage.newest_observed,
        Some(ts("2026-09-22T00:00:00Z"))
    );

    assert_eq!(
        ActivityWindow::new(ts("2026-09-20T00:00:00Z"), ts("2026-09-20T00:00:00Z")),
        None
    );
    assert_eq!(
        ActivityWindow::new(ts("2026-09-21T00:00:00Z"), ts("2026-09-20T00:00:00Z")),
        None
    );
}

#[test]
fn only_the_latest_rows_are_returned_but_all_are_counted() {
    let snapshot = Source::with(&rows(150)).snapshot();
    assert_eq!(snapshot.rows.len(), ReadLimits::MAX_ROWS);
    assert!(snapshot.coverage.row_bound_reached);
    assert_eq!(snapshot.counts.distinct, 150);
    assert_eq!(snapshot.counts.in_window.advise, 150);
    assert_eq!(snapshot.rows[0].fire_id, "f00149");
    assert_eq!(snapshot.rows[99].fire_id, "f00050");
    // The presentation bound does not make the counts inexact.
    assert_eq!(snapshot.counts.precision, Precision::Exact);

    let limits = ReadLimits::default().tighten_rows(2);
    let snapshot = expect_snapshot(Source::with(&rows(2)).read_with(&limits, None));
    assert!(!snapshot.coverage.row_bound_reached);
    assert_eq!(snapshot.rows.len(), 2);
}

// ── observed groups ────────────────────────────────────────────────────────

/// A full plugin-shaped row (with its secret-bearing excluded fields) for
/// `rule` under `book`; `None` removes `rulebook_id`.
fn fire(fire_id: &str, book: Option<&str>, rule: &str, fired_at: &str, mode: &str) -> String {
    let mut row = row_value(fire_id, fired_at, mode);
    row["rule_id"] = json!(rule);
    match book {
        Some(book) => row["rulebook_id"] = json!(book),
        None => {
            row.as_object_mut().unwrap().remove("rulebook_id");
        }
    }
    line(&row)
}

/// `(rulebook_id, rule_id)` of each returned group, in order.
fn group_keys(snapshot: &ActivitySnapshot) -> Vec<(Option<&str>, &str)> {
    snapshot
        .groups
        .iter()
        .map(|g| (g.rulebook_id.as_deref(), g.rule_id.as_str()))
        .collect()
}

fn group<'a>(snapshot: &'a ActivitySnapshot, book: Option<&str>, rule: &str) -> &'a ObservedGroup {
    snapshot
        .groups
        .iter()
        .find(|g| g.rulebook_id.as_deref() == book && g.rule_id == rule)
        .unwrap_or_else(|| panic!("no group {book:?}/{rule}"))
}

/// `groups` groups `rule-00000`… one minute apart, oldest first, with
/// `per_group` fires each.
fn grouped_rows(groups: usize, per_group: usize) -> String {
    let base = ts("2026-09-01T00:00:00Z");
    (0..groups)
        .flat_map(|g| (0..per_group).map(move |f| (g, f)))
        .map(|(g, f)| {
            let at = base + jiff::SignedDuration::from_secs(60 * g as i64);
            fire(
                &format!("f{g:05}-{f}"),
                Some("book-1"),
                &format!("rule-{g:05}"),
                &at.to_string(),
                "advise",
            )
        })
        .collect()
}

#[test]
fn groups_cover_every_in_window_row_not_only_the_returned_rows() {
    let base = ts("2026-09-01T00:00:00Z");
    let mut ledger: String = (0..150)
        .map(|i| {
            let at = base + jiff::SignedDuration::from_secs(60 * i);
            fire(
                &format!("a{i:03}"),
                Some("book-1"),
                "rule-a",
                &at.to_string(),
                "gate",
            )
        })
        .collect();
    ledger += &fire(
        "b",
        Some("book-1"),
        "rule-b",
        "2026-09-02T00:00:00Z",
        "advise",
    );
    let read = Source::with(&ledger).read();
    assert_no_secrets(&read);
    let snapshot = expect_snapshot(read);

    assert_eq!(snapshot.rows.len(), 100);
    assert!(snapshot.coverage.row_bound_reached);
    assert!(!snapshot.coverage.group_bound_reached);
    assert_eq!(snapshot.counts.in_window.total(), 151);
    assert_eq!(snapshot.counts.observed_groups, 2);
    assert_eq!(
        group_keys(&snapshot),
        [(Some("book-1"), "rule-b"), (Some("book-1"), "rule-a")]
    );
    assert_eq!(
        group(&snapshot, Some("book-1"), "rule-a").modes,
        ModeCounts {
            gate: 150,
            ..ModeCounts::default()
        }
    );
    assert_eq!(
        group(&snapshot, Some("book-1"), "rule-a").latest_observed,
        base + jiff::SignedDuration::from_secs(60 * 149)
    );
    assert_eq!(group(&snapshot, Some("book-1"), "rule-b").modes.advise, 1);
    // The presentation bounds do not make the counts inexact.
    assert_eq!(snapshot.counts.precision, Precision::Exact);
}

#[test]
fn only_the_latest_groups_are_returned_but_all_are_counted() {
    let snapshot = Source::with(&grouped_rows(150, 2)).snapshot();
    assert_eq!(snapshot.counts.in_window.advise, 300);
    assert_eq!(snapshot.counts.observed_groups, 150);
    assert_eq!(snapshot.groups.len(), ReadLimits::MAX_GROUPS);
    assert!(snapshot.coverage.group_bound_reached);
    assert_eq!(snapshot.rows.len(), ReadLimits::MAX_ROWS);
    assert!(snapshot.coverage.row_bound_reached);
    assert_eq!(snapshot.groups[0].rule_id, "rule-00149");
    assert_eq!(snapshot.groups[99].rule_id, "rule-00050");
    // Returned groups keep their full counts; the bound only drops groups.
    assert!(snapshot.groups.iter().all(|g| g.modes.advise == 2));
    assert_eq!(snapshot.counts.precision, Precision::Exact);

    // The group bound tightens independently of the row bound.
    let limits = ReadLimits::default().tighten_groups(3);
    let snapshot = expect_snapshot(Source::with(&grouped_rows(5, 1)).read_with(&limits, None));
    assert_eq!(snapshot.rows.len(), 5);
    assert!(!snapshot.coverage.row_bound_reached);
    assert_eq!(snapshot.groups.len(), 3);
    assert!(snapshot.coverage.group_bound_reached);
    assert_eq!(snapshot.counts.observed_groups, 5);

    let limits = ReadLimits::default().tighten_groups(5);
    let snapshot = expect_snapshot(Source::with(&grouped_rows(5, 1)).read_with(&limits, None));
    assert_eq!(snapshot.groups.len(), 5);
    assert!(!snapshot.coverage.group_bound_reached);
}

#[test]
fn duplicates_and_conflicts_are_resolved_before_grouping() {
    let window = ActivityWindow::new(ts("2026-09-20T00:00:00Z"), ts("2026-09-21T00:00:00Z"));
    // An identical row and a row differing only in excluded content are
    // one fire.
    let dup = fire(
        "dup",
        Some("book-1"),
        "rule-a",
        "2026-09-20T10:00:00Z",
        "gate",
    );
    let mut excluded_only = row_value("dup", "2026-09-20T10:00:00Z", "gate");
    excluded_only["rule_id"] = json!("rule-a");
    excluded_only["excerpt"] = json!("another excerpt");
    excluded_only["override_reason"] = json!("another reason");
    let ledger = [
        dup.clone(),
        dup,
        line(&excluded_only),
        // One fire_id with an in-window row and a conflicting row outside
        // the window: excluded from groups entirely.
        fire(
            "split",
            Some("book-1"),
            "rule-b",
            "2026-09-20T11:00:00Z",
            "gate",
        ),
        fire(
            "split",
            Some("book-1"),
            "rule-b",
            "2026-09-19T11:00:00Z",
            "gate",
        ),
        // A conflict on the group key itself counts for neither group.
        fire(
            "moved",
            Some("book-1"),
            "rule-a",
            "2026-09-20T12:00:00Z",
            "advise",
        ),
        fire(
            "moved",
            Some("book-2"),
            "rule-a",
            "2026-09-20T12:00:00Z",
            "advise",
        ),
    ]
    .concat();
    let snapshot = expect_snapshot(Source::with(&ledger).read_with(&ReadLimits::default(), window));
    assert_eq!(snapshot.counts.duplicate_rows, 2);
    assert_eq!(snapshot.counts.conflicted_ids, 2);
    assert_eq!(snapshot.counts.conflicted_rows, 4);
    assert_eq!(snapshot.counts.observed_groups, 1);
    assert_eq!(group_keys(&snapshot), [(Some("book-1"), "rule-a")]);
    assert_eq!(
        snapshot.groups[0].modes,
        ModeCounts {
            gate: 1,
            ..ModeCounts::default()
        }
    );
    assert_eq!(
        snapshot.groups[0].latest_observed,
        ts("2026-09-20T10:00:00Z")
    );
    assert_eq!(snapshot.counts.precision, Precision::LowerBound);
}

#[test]
fn rulebooks_scope_groups_and_missing_rulebooks_stay_unscoped() {
    let null_book = {
        let mut row = row_value("null-book", "2026-09-20T10:00:05Z", "advise");
        row["rule_id"] = json!("rule-a");
        row["rulebook_id"] = Value::Null;
        line(&row)
    };
    let ledger = [
        fire(
            "b1",
            Some("book-1"),
            "rule-a",
            "2026-09-20T10:00:01Z",
            "advise",
        ),
        fire(
            "b1-again",
            Some("book-1"),
            "rule-a",
            "2026-09-20T10:00:02Z",
            "advise",
        ),
        fire(
            "b2",
            Some("book-2"),
            "rule-a",
            "2026-09-20T10:00:03Z",
            "advise",
        ),
        fire("none", None, "rule-a", "2026-09-20T10:00:04Z", "advise"),
        null_book,
        // Raw identities: no trimming, case folding or empty-as-missing.
        fire(
            "padded",
            Some(" book-1"),
            "rule-a",
            "2026-09-20T10:00:06Z",
            "advise",
        ),
        fire(
            "empty",
            Some(""),
            "rule-a",
            "2026-09-20T10:00:07Z",
            "advise",
        ),
        fire(
            "case",
            Some("book-1"),
            "Rule-A",
            "2026-09-20T10:00:08Z",
            "advise",
        ),
    ]
    .concat();
    let snapshot = Source::with(&ledger).snapshot();
    assert_eq!(snapshot.counts.observed_groups, 6);
    assert_eq!(
        group_keys(&snapshot),
        [
            (Some("book-1"), "Rule-A"),
            (Some(""), "rule-a"),
            (Some(" book-1"), "rule-a"),
            (None, "rule-a"),
            (Some("book-2"), "rule-a"),
            (Some("book-1"), "rule-a"),
        ]
    );
    // Absent and null rulebooks are one unscoped group, apart from every
    // scoped group of the same rule.
    assert_eq!(group(&snapshot, None, "rule-a").modes.advise, 2);
    assert_eq!(group(&snapshot, Some("book-1"), "rule-a").modes.advise, 2);
    assert_eq!(group(&snapshot, Some("book-2"), "rule-a").modes.advise, 1);
}

#[test]
fn a_group_keeps_every_mode_and_version_without_choosing_one() {
    let versioned = |fire_id: &str, at: &str, mode: &str, version: Value| {
        let mut row = row_value(fire_id, at, mode);
        row["rule_version"] = version;
        line(&row)
    };
    let ledger = [
        versioned("v3-advise", "2026-09-20T10:00:00Z", "advise", json!(3)),
        versioned("v4-gate", "2026-09-20T11:00:00Z", "gate", json!(4)),
        versioned(
            "label-suppressed",
            "2026-09-20T12:00:00Z",
            "suppressed",
            json!("v5-draft"),
        ),
        versioned(
            "unversioned-shadow",
            "2026-09-20T13:00:00Z",
            "shadow",
            Value::Null,
        ),
    ]
    .concat();
    let snapshot = Source::with(&ledger).snapshot();
    assert_eq!(snapshot.counts.observed_groups, 1);
    let group = &snapshot.groups[0];
    assert_eq!(
        group.modes,
        ModeCounts {
            advise: 1,
            gate: 1,
            suppressed: 1,
            unrecognized: 1,
        }
    );
    // Suppressed and unknown modes are not recorded fires.
    assert_eq!(group.modes.recorded_fires(), 2);
    assert_eq!(group.latest_observed, ts("2026-09-20T13:00:00Z"));
    // Each row keeps its own observed version and raw mode.
    let versions: Vec<_> = snapshot
        .rows
        .iter()
        .map(|r| r.rule_version.clone())
        .collect();
    assert_eq!(
        versions,
        [
            None,
            Some(RuleVersion::Label("v5-draft".into())),
            Some(RuleVersion::Number(4)),
            Some(RuleVersion::Number(3)),
        ]
    );
    assert_eq!(
        snapshot.rows[0].mode,
        FireMode::Unrecognized("shadow".into())
    );
}

#[test]
fn groups_use_only_in_window_rows() {
    let window = ActivityWindow::new(ts("2026-09-20T00:00:00Z"), ts("2026-09-21T00:00:00Z"));
    let ledger = [
        fire(
            "before",
            Some("book-1"),
            "rule-out",
            "2026-09-19T23:59:59Z",
            "gate",
        ),
        fire(
            "at-end",
            Some("book-1"),
            "rule-out",
            "2026-09-21T00:00:00Z",
            "gate",
        ),
        fire(
            "at-start",
            Some("book-1"),
            "rule-in",
            "2026-09-20T00:00:00Z",
            "gate",
        ),
        fire(
            "inside",
            Some("book-1"),
            "rule-in",
            "2026-09-20T09:00:00Z",
            "advise",
        ),
        // A newer out-of-window fire does not move the group's latest.
        fire(
            "later",
            Some("book-1"),
            "rule-in",
            "2026-09-22T00:00:00Z",
            "advise",
        ),
    ]
    .concat();
    let snapshot = expect_snapshot(Source::with(&ledger).read_with(&ReadLimits::default(), window));
    assert_eq!(snapshot.counts.observed_groups, 1);
    assert_eq!(group_keys(&snapshot), [(Some("book-1"), "rule-in")]);
    assert_eq!(
        snapshot.groups[0].modes,
        ModeCounts {
            advise: 1,
            gate: 1,
            ..ModeCounts::default()
        }
    );
    assert_eq!(
        snapshot.groups[0].latest_observed,
        ts("2026-09-20T09:00:00Z")
    );

    // A window with no rows yields no groups; without a window every
    // accepted row is grouped.
    let empty = ActivityWindow::new(ts("2026-01-01T00:00:00Z"), ts("2026-01-02T00:00:00Z"));
    let snapshot = expect_snapshot(Source::with(&ledger).read_with(&ReadLimits::default(), empty));
    assert!(snapshot.groups.is_empty());
    assert_eq!(snapshot.counts.observed_groups, 0);
    let snapshot = Source::with(&ledger).snapshot();
    assert_eq!(snapshot.counts.observed_groups, 2);
}

#[test]
fn group_ties_order_by_rulebook_then_rule_and_are_stable() {
    let at = "2026-09-20T10:00:00Z";
    let ledger = [
        fire("1", Some("book-b"), "rule-a", at, "gate"),
        fire("2", Some("book-a"), "rule-b", at, "gate"),
        fire("3", None, "rule-z", at, "gate"),
        fire("4", Some("book-a"), "rule-a", at, "gate"),
        // The same instant written with another offset ties too.
        fire("5", None, "rule-a", "2026-09-20T12:00:00+02:00", "gate"),
        fire(
            "6",
            Some("book-c"),
            "rule-newer",
            "2026-09-20T10:00:01Z",
            "gate",
        ),
    ]
    .concat();
    let source = Source::with(&ledger);
    let expected = [
        (Some("book-c"), "rule-newer"),
        (None, "rule-a"),
        (None, "rule-z"),
        (Some("book-a"), "rule-a"),
        (Some("book-a"), "rule-b"),
        (Some("book-b"), "rule-a"),
    ];
    for _ in 0..8 {
        assert_eq!(group_keys(&source.snapshot()), expected);
    }
    // A tie at the bound is cut by the same order.
    let limits = ReadLimits::default().tighten_groups(3);
    let snapshot = expect_snapshot(source.read_with(&limits, None));
    assert_eq!(group_keys(&snapshot), expected[..3]);
}

// ── byte, line and partial-line bounds ─────────────────────────────────────

#[test]
fn a_tail_start_inside_a_line_drops_that_line() {
    let ledger = rows(10);
    let line_len = ledger.len() / 10;
    let limits = ReadLimits::default().tighten_tail_bytes((line_len * 3 + 5) as u64);
    let snapshot = expect_snapshot(Source::with(&ledger).read_with(&limits, None));
    assert!(snapshot.coverage.byte_bound_reached);
    assert!(snapshot.coverage.leading_partial_dropped);
    assert_eq!(snapshot.counts.lines, 3);
    assert_eq!(snapshot.counts.malformed.total(), 0);
    assert_eq!(
        snapshot.coverage.scanned,
        (line_len * 7) as u64..ledger.len() as u64
    );
    assert_eq!(snapshot.rows.last().unwrap().fire_id, "f00007");
    assert_eq!(snapshot.counts.precision, Precision::LowerBound);
    // The earliest observed row is where the bound cut, not coverage start.
    assert_eq!(
        snapshot.coverage.oldest_observed,
        Some(snapshot.rows[2].fired_at)
    );
}

#[test]
fn a_tail_start_on_a_line_boundary_keeps_the_first_line() {
    let ledger = rows(10);
    let line_len = ledger.len() / 10;
    let limits = ReadLimits::default().tighten_tail_bytes((line_len * 3) as u64);
    let snapshot = expect_snapshot(Source::with(&ledger).read_with(&limits, None));
    assert!(snapshot.coverage.byte_bound_reached);
    assert!(!snapshot.coverage.leading_partial_dropped);
    assert_eq!(snapshot.counts.lines, 3);
    assert_eq!(snapshot.counts.precision, Precision::LowerBound);
}

#[test]
fn a_tail_entirely_inside_one_line_yields_no_rows() {
    let ledger = rows(3);
    let limits = ReadLimits::default().tighten_tail_bytes(10);
    let snapshot = expect_snapshot(Source::with(&ledger).read_with(&limits, None));
    assert!(snapshot.coverage.leading_partial_dropped);
    assert_eq!(snapshot.counts.lines, 0);
    assert!(snapshot.rows.is_empty());
    assert_eq!(snapshot.counts.precision, Precision::LowerBound);
}

#[test]
fn an_unfinished_final_line_is_dropped_and_reported() {
    let complete = rows(2);
    let unfinished = row("f99999", "2026-09-30T00:00:00Z", "gate");
    let ledger = format!("{complete}{}", &unfinished[..unfinished.len() / 2]);
    let snapshot = Source::with(&ledger).snapshot();
    assert!(snapshot.coverage.trailing_partial_dropped);
    assert!(!snapshot.coverage.byte_bound_reached);
    assert_eq!(snapshot.counts.lines, 2);
    assert_eq!(snapshot.counts.malformed.total(), 0);
    assert_eq!(snapshot.coverage.scanned, 0..complete.len() as u64);
    assert_eq!(snapshot.rows.len(), 2);
    // Groups of the complete rows carry the same (lower-bound) precision.
    assert_eq!(snapshot.counts.observed_groups, 1);
    assert_eq!(snapshot.groups[0].modes.advise, 2);
    assert_eq!(snapshot.counts.precision, Precision::LowerBound);

    // A complete row without its newline is still unfinished.
    let snapshot = Source::with(unfinished.trim_end()).snapshot();
    assert!(snapshot.coverage.trailing_partial_dropped);
    assert!(snapshot.rows.is_empty());
}

#[test]
fn only_the_latest_lines_are_examined() {
    let ledger = rows(10);
    let limits = ReadLimits::default().tighten_lines(3);
    let snapshot = expect_snapshot(Source::with(&ledger).read_with(&limits, None));
    assert!(snapshot.coverage.line_bound_reached);
    assert!(!snapshot.coverage.byte_bound_reached);
    assert_eq!(snapshot.counts.lines, 3);
    let line_len = (ledger.len() / 10) as u64;
    assert_eq!(snapshot.coverage.scanned, line_len * 7..line_len * 10);
    let ids: Vec<_> = snapshot.rows.iter().map(|r| r.fire_id.as_str()).collect();
    assert_eq!(ids, ["f00009", "f00008", "f00007"]);
    assert_eq!(snapshot.counts.precision, Precision::LowerBound);
}

#[test]
fn a_tail_of_only_newlines_keeps_the_latest_lines_within_bounds() {
    let len = ReadLimits::MAX_TAIL_BYTES as usize + 100;
    let source = Source::with(&"\n".repeat(len));
    let snapshot = source.snapshot();
    assert!(snapshot.coverage.byte_bound_reached);
    assert!(!snapshot.coverage.leading_partial_dropped);
    assert!(snapshot.coverage.line_bound_reached);
    assert_eq!(snapshot.counts.lines, ReadLimits::MAX_LINES as u64);
    assert_eq!(snapshot.counts.blank_lines, ReadLimits::MAX_LINES as u64);
    let end = len as u64;
    assert_eq!(
        snapshot.coverage.scanned,
        end - ReadLimits::MAX_LINES as u64..end
    );
    assert_eq!(snapshot.counts.precision, Precision::LowerBound);
}

#[test]
fn a_zero_line_bound_examines_nothing() {
    let ledger = rows(3);
    let limits = ReadLimits::default().tighten_lines(0);
    let snapshot = expect_snapshot(Source::with(&ledger).read_with(&limits, None));
    assert!(snapshot.coverage.line_bound_reached);
    assert_eq!(snapshot.counts.lines, 0);
    let end = ledger.len() as u64;
    assert_eq!(snapshot.coverage.scanned, end..end);
    assert!(snapshot.rows.is_empty());
    assert_eq!(snapshot.counts.precision, Precision::LowerBound);

    // Nothing to exceed: an empty ledger under a zero bound stays exact.
    let snapshot = expect_snapshot(Source::with("").read_with(&limits, None));
    assert!(!snapshot.coverage.line_bound_reached);
    assert_eq!(snapshot.counts.precision, Precision::Exact);
}

#[test]
fn the_default_line_bound_is_ten_thousand_lines() {
    let at_bound = Source::with(&compact_rows(ReadLimits::MAX_LINES)).snapshot();
    assert!(!at_bound.coverage.line_bound_reached);
    assert_eq!(at_bound.counts.distinct, 10_000);
    assert_eq!(at_bound.counts.precision, Precision::Exact);

    let ledger = compact_rows(ReadLimits::MAX_LINES + 1);
    assert!((ledger.len() as u64) < ReadLimits::MAX_TAIL_BYTES);
    let over = Source::with(&ledger).snapshot();
    assert!(over.coverage.line_bound_reached);
    assert!(!over.coverage.byte_bound_reached);
    assert_eq!(over.counts.lines, 10_000);
    assert_eq!(over.counts.distinct, 10_000);
    assert_eq!(over.rows[0].fire_id, "f10000");
    assert_eq!(over.rows.last().unwrap().fire_id, "f09901");
    assert_eq!(over.counts.precision, Precision::LowerBound);
}

#[test]
fn the_default_byte_bound_is_four_mebibytes() {
    // 80 lines of ~64 KiB exceed 4 MiB; only the latest fit in the tail.
    let padding = "p".repeat(60 << 10);
    let ledger: String = (0..80)
        .map(|i| {
            let mut row = row_value(&format!("big{i:03}"), "2026-09-20T10:00:00Z", "gate");
            row["padding"] = json!(padding);
            line(&row)
        })
        .collect();
    assert!(ledger.len() as u64 > ReadLimits::MAX_TAIL_BYTES);
    let snapshot = Source::with(&ledger).snapshot();
    assert!(snapshot.coverage.byte_bound_reached);
    assert!(
        snapshot.coverage.scanned.end - snapshot.coverage.scanned.start
            <= ReadLimits::MAX_TAIL_BYTES
    );
    assert!(snapshot.counts.distinct < 80);
    assert!(snapshot.rows.iter().any(|r| r.fire_id == "big079"));
    assert!(!snapshot.rows.iter().any(|r| r.fire_id == "big000"));
    assert_eq!(snapshot.counts.precision, Precision::LowerBound);
}

#[test]
fn limits_only_tighten() {
    let defaults = ReadLimits::default();
    assert_eq!(defaults.tail_bytes(), 4 * 1024 * 1024);
    assert_eq!(defaults.lines(), 10_000);
    assert_eq!(defaults.line_bytes(), 64 * 1024);
    assert_eq!(defaults.value_bytes(), 512);
    assert_eq!(defaults.rows(), 100);
    assert_eq!(defaults.groups(), 100);
    assert_eq!(defaults.deadline(), Duration::from_secs(1));
    let loosened = defaults
        .tighten_tail_bytes(u64::MAX)
        .tighten_lines(usize::MAX)
        .tighten_line_bytes(usize::MAX)
        .tighten_value_bytes(usize::MAX)
        .tighten_rows(usize::MAX)
        .tighten_groups(usize::MAX)
        .tighten_deadline(Duration::from_secs(3600));
    assert_eq!(loosened, defaults);
    let tightened = defaults.tighten_lines(5).tighten_lines(50);
    assert_eq!(tightened.lines(), 5);
}

// ── snapshot races ─────────────────────────────────────────────────────────

fn read_mutating(source: &Source, mutate: impl FnOnce(&Source)) -> ActivityRead {
    let mut mutate = Some(mutate);
    read_activity_with(
        &source.root,
        &ReadLimits::default(),
        None,
        &|| false,
        &mut Checkpoints {
            after_capture: &mut || (mutate.take().unwrap())(source),
            after_assembly: &mut || {},
        },
    )
}

#[test]
fn an_append_after_capture_is_newer_than_the_snapshot() {
    let source = Source::with(&rows(3));
    let captured = rows(3).len() as u64;
    let read = read_mutating(&source, |s| {
        s.append(row("late", "2026-09-30T00:00:00Z", "gate").as_bytes());
    });
    let snapshot = expect_snapshot(read);
    assert!(snapshot.coverage.grew_after_capture);
    assert_eq!(snapshot.coverage.captured_len, captured);
    assert_eq!(snapshot.counts.distinct, 3);
    assert!(!snapshot.rows.iter().any(|r| r.fire_id == "late"));
    // The append does not make the captured snapshot inexact.
    assert_eq!(snapshot.counts.precision, Precision::Exact);
}

#[test]
fn truncation_during_the_read_discards_the_snapshot() {
    let source = Source::with(&rows(3));
    let read = read_mutating(&source, |s| {
        fs::OpenOptions::new()
            .write(true)
            .open(s.ledger_path())
            .unwrap()
            .set_len(10)
            .unwrap();
    });
    assert_eq!(read, ActivityRead::SourceChanged(SourceChange::Shrunk));

    // An in-place rewrite that leaves the file shorter is truncation too.
    let source = Source::with(&rows(3));
    let read = read_mutating(&source, |s| s.ledger(rows(1).as_bytes()));
    assert_eq!(read, ActivityRead::SourceChanged(SourceChange::Shrunk));
}

#[test]
fn replacement_or_removal_during_the_read_discards_the_snapshot() {
    let source = Source::with(&rows(3));
    let read = read_mutating(&source, |s| {
        let next = s.root.join("ledger/next.jsonl");
        fs::write(&next, rows(5)).unwrap();
        fs::rename(&next, s.ledger_path()).unwrap();
    });
    assert_eq!(read, ActivityRead::SourceChanged(SourceChange::Replaced));

    let source = Source::with(&rows(3));
    let read = read_mutating(&source, |s| fs::remove_file(s.ledger_path()).unwrap());
    assert_eq!(read, ActivityRead::SourceChanged(SourceChange::Removed));

    #[cfg(unix)]
    {
        let source = Source::with(&rows(3));
        let read = read_mutating(&source, |s| {
            let kept = s.root.join("ledger/kept.jsonl");
            fs::rename(s.ledger_path(), &kept).unwrap();
            std::os::unix::fs::symlink(&kept, s.ledger_path()).unwrap();
        });
        assert_eq!(read, ActivityRead::SourceChanged(SourceChange::Replaced));
    }
}

#[test]
fn each_read_is_a_fresh_snapshot_never_merged_across_rotation() {
    let source = Source::with(&rows(3));
    let first = source.snapshot();
    assert_eq!(first.counts.distinct, 3);

    // Rotation: a new file object with different rows.
    let next = source.root.join("ledger/next.jsonl");
    fs::write(&next, row("fresh", "2026-09-25T00:00:00Z", "advise")).unwrap();
    fs::rename(&next, source.ledger_path()).unwrap();
    let second = source.snapshot();
    assert_eq!(second.counts.distinct, 1);
    assert_eq!(second.rows[0].fire_id, "fresh");
    assert!(!first.same_source_object(&second));

    // Truncation in place keeps the object but not the rows.
    source.ledger(row("after", "2026-09-26T00:00:00Z", "gate").as_bytes());
    let third = source.snapshot();
    assert_eq!(third.counts.distinct, 1);
    assert_eq!(third.rows[0].fire_id, "after");
    assert!(second.same_source_object(&third));
}

// ── deadline and cancellation ──────────────────────────────────────────────

#[test]
fn cancellation_interrupts_without_partial_counts() {
    let source = Source::with(&rows(2_000));
    let read = read_activity(&source.root, &ReadLimits::default(), None, &|| true);
    assert_eq!(read, ActivityRead::Interrupted(Interruption::Cancelled));

    // Cancelled later in the read, after several polls have passed.
    let polls = Cell::new(0);
    let cancel = || {
        polls.set(polls.get() + 1);
        polls.get() > 10
    };
    let read = read_activity(&source.root, &ReadLimits::default(), None, &cancel);
    assert_eq!(read, ActivityRead::Interrupted(Interruption::Cancelled));
    assert_eq!(polls.get(), 11);
}

/// Read with a hook that runs once the snapshot is fully assembled, just
/// before the final budget check.
fn read_finalizing(
    source: &Source,
    limits: &ReadLimits,
    cancel: &dyn Fn() -> bool,
    at_assembly: &mut dyn FnMut(),
) -> ActivityRead {
    read_activity_with(
        &source.root,
        limits,
        None,
        cancel,
        &mut Checkpoints {
            after_capture: &mut || {},
            after_assembly: at_assembly,
        },
    )
}

#[test]
fn cancellation_during_finalization_yields_no_snapshot() {
    let source = Source::with(&rows(300));
    let cancelled = Cell::new(false);
    let polls_before = Cell::new(0);
    let cancel = || {
        if !cancelled.get() {
            polls_before.set(polls_before.get() + 1);
        }
        cancelled.get()
    };
    let limits = ReadLimits::default();
    let read = read_finalizing(&source, &limits, &cancel, &mut || cancelled.set(true));
    assert_eq!(read, ActivityRead::Interrupted(Interruption::Cancelled));
    // Every earlier check passed: only the final check could interrupt.
    assert!(polls_before.get() > 0);

    // The same read without a late cancellation completes exactly.
    let read = read_finalizing(&source, &limits, &|| false, &mut || {});
    assert_eq!(expect_snapshot(read).counts.precision, Precision::Exact);
}

#[test]
fn a_deadline_passing_during_finalization_yields_no_snapshot() {
    let source = Source::with(&rows(3));
    let limits = ReadLimits::default().tighten_deadline(Duration::from_millis(250));
    let reached = Cell::new(false);
    let read = read_finalizing(&source, &limits, &|| false, &mut || {
        reached.set(true);
        std::thread::sleep(Duration::from_millis(300));
    });
    assert!(reached.get(), "an earlier check expired before assembly");
    assert_eq!(
        read,
        ActivityRead::Interrupted(Interruption::DeadlineExceeded)
    );
}

#[test]
fn grouping_is_inside_the_final_budget_check() {
    // Groups are assembled before the final check, so a cancellation or
    // deadline there discards them with everything else.
    let source = Source::with(&grouped_rows(150, 2));
    let cancelled = Cell::new(false);
    let limits = ReadLimits::default();
    let read = read_finalizing(&source, &limits, &|| cancelled.get(), &mut || {
        cancelled.set(true)
    });
    assert_eq!(read, ActivityRead::Interrupted(Interruption::Cancelled));

    let limits = ReadLimits::default().tighten_deadline(Duration::from_millis(250));
    let read = read_finalizing(&source, &limits, &|| false, &mut || {
        std::thread::sleep(Duration::from_millis(300));
    });
    assert_eq!(
        read,
        ActivityRead::Interrupted(Interruption::DeadlineExceeded)
    );

    let read = read_finalizing(&source, &ReadLimits::default(), &|| false, &mut || {});
    assert_eq!(expect_snapshot(read).counts.observed_groups, 150);
}

#[test]
fn an_expired_deadline_interrupts_without_partial_counts() {
    let source = Source::with(&rows(10));
    let limits = ReadLimits::default().tighten_deadline(Duration::ZERO);
    assert_eq!(
        source.read_with(&limits, None),
        ActivityRead::Interrupted(Interruption::DeadlineExceeded)
    );
}

// ── read-only and hygiene ──────────────────────────────────────────────────

#[test]
fn reads_never_modify_the_source_tree() {
    let source = Source::with(&(rows(20) + "{broken\n" + &rows(1)[..40]));
    let home = source
        .root
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let before = tree(&home);
    for limits in [
        ReadLimits::default(),
        ReadLimits::default().tighten_tail_bytes(300),
        ReadLimits::default().tighten_lines(2),
        ReadLimits::default().tighten_deadline(Duration::ZERO),
    ] {
        let _ = source.read_with(&limits, None);
    }
    let _ = read_activity(&source.root, &ReadLimits::default(), None, &|| true);
    assert_eq!(tree(&home), before);

    // Failed reads leave their trees untouched too.
    let bare = Source::bare();
    let bare_home = bare.root.parent().unwrap().parent().unwrap().to_path_buf();
    let before = tree(&bare_home);
    let _ = bare.read();
    assert_eq!(tree(&bare_home), before);
}

#[test]
fn the_reader_has_no_output_process_network_or_write_paths() {
    let sources = [
        include_str!("../activity.rs"),
        include_str!("record.rs"),
        include_str!("source.rs"),
    ];
    let forbidden = [
        "println!",
        "eprintln!",
        "print!",
        "dbg!",
        "log::",
        "tracing",
        "std::process",
        "Command",
        "std::net",
        "TcpStream",
        ".write(true)",
        ".append(true)",
        ".create(true)",
        "fs::write",
        "create_dir",
        "remove_file",
        "rename",
        "set_len",
        "env::var",
        "home_dir",
    ];
    for source in sources {
        for token in forbidden {
            assert!(!source.contains(token), "reader source contains {token}");
        }
    }
}

#[test]
fn impossible_instants_are_invalid_timestamps() {
    let ledger = [
        with_field("a", "fired_at", json!("2026-02-30T10:00:00Z")),
        with_field("b", "fired_at", json!("2026-09-20T24:00:01Z")),
        with_field("c", "fired_at", json!("2026-09-20T10:61:00Z")),
        with_field("d", "fired_at", json!("2026-09-20T10:00:00+25:00")),
        with_field("e", "fired_at", json!("2026-09-20T10:00:00+24:00")),
        with_field("f", "fired_at", json!("2026-09-20T10:00:00+05:60")),
        with_field("g", "fired_at", json!("2026-09-20T10:00:00+0530")),
        with_field("h", "fired_at", json!("2026-09-20T10:00:00+05")),
        with_field("i", "fired_at", json!("2026-09-20 10:00:00Z")),
        with_field("j", "fired_at", json!("2026-09-20T10:00Z")),
        with_field("k", "fired_at", json!("2026-09-20T10:00:00.Z")),
        with_field("l", "fired_at", json!("2026-09-20T10:00:00.0000000001Z")),
        with_field("m", "fired_at", json!("2026-09-20T10:00:00Z[UTC]")),
        with_field("n", "fired_at", json!("+002026-09-20T10:00:00Z")),
        with_field("o", "fired_at", json!("2026-09-20T10:00:00Z ")),
    ]
    .concat();
    let accepted = [
        with_field(
            "ok-plugin",
            "fired_at",
            json!("2026-09-20T10:00:00.000001+02:00"),
        ),
        with_field("ok-z", "fired_at", json!("2026-09-20t10:00:00z")),
        with_field("ok-west", "fired_at", json!("2026-09-20T10:00:00-23:59")),
        with_field(
            "ok-nanos",
            "fired_at",
            json!("2026-09-20T10:00:00.123456789Z"),
        ),
    ]
    .concat();
    let snapshot = Source::with(&(ledger + &accepted)).snapshot();
    assert_eq!(snapshot.counts.malformed.invalid_timestamp, 15);
    assert_eq!(snapshot.counts.malformed.total(), 15);
    let mut ids: Vec<_> = snapshot.rows.iter().map(|r| r.fire_id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["ok-nanos", "ok-plugin", "ok-west", "ok-z"]);
}
