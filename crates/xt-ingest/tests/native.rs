//! The shared reader stream (header + canonical records) imported through the
//! canonical writer: identity, surface, start, cwd, branch, usage and counts are
//! preserved metadata-only; repeated import deduplicates; malformed sessions
//! are skipped explicitly while the rest import; and each unavailable
//! prerequisite for the reader hosts yields its own explicit status.
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use xt_ingest::native::{
    HostStatus, ImportRequest, SessionOutcome, import_block, import_native,
    stream::{BlockOutcome, StreamError, parse_stream},
};
use xt_store::{Host, SessionSource, Store, batch::SourceCursor};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn golden(fixture: &str, key: &str) -> Vec<String> {
    let text =
        fs::read_to_string(repo().join(format!("fixtures/{fixture}/input/native/golden.json")))
            .unwrap();
    let golden: Value = serde_json::from_str(&text).unwrap();
    golden[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap().to_owned())
        .collect()
}

fn expected(fixture: &str) -> Vec<Value> {
    let text =
        fs::read_to_string(repo().join(format!("fixtures/{fixture}/input/native/native.json")))
            .unwrap();
    serde_json::from_str::<Value>(&text).unwrap()["expected_headers"]
        .as_array()
        .unwrap()
        .clone()
}

fn is_header(line: &str) -> bool {
    serde_json::from_str::<Value>(line).unwrap()["type"] == "session"
}

fn rekey(line: &str, from: &str, to: &str) -> String {
    let mut value: Value = serde_json::from_str(line).unwrap();
    let object = value.as_object_mut().unwrap();
    let inner = object.remove(from).unwrap();
    object.insert(to.into(), inner);
    value.to_string()
}

fn reset(line: &str, key: &str, new: &str) -> String {
    let mut value: Value = serde_json::from_str(line).unwrap();
    value[key] = Value::String(new.into());
    value.to_string()
}

fn import(
    store: &mut Store,
    host: Host,
    lines: &[String],
) -> Vec<xt_ingest::native::SessionResult> {
    parse_stream(lines, host, SessionSource::ReadersCli)
        .unwrap()
        .into_iter()
        .map(|block| {
            let cursor = match &block {
                BlockOutcome::Ready(ready) => Some(SourceCursor {
                    source: SessionSource::ReadersCli,
                    cursor_key: format!("{}:{}", host.as_str(), ready.header.path),
                    position: (ready.header.mtime * 1000.0) as i64,
                    updated_at: 1,
                }),
                BlockOutcome::Malformed { .. } => None,
            };
            import_block(
                store,
                host,
                SessionSource::ReadersCli,
                block,
                cursor,
                1_788_782_400_000,
            )
        })
        .collect()
}

#[test]
fn reader_streams_import_identity_surface_usage_and_counts_metadata_only() {
    let mut store = Store::open_in_memory().unwrap();
    for host in [Host::Codex, Host::Cursor] {
        let results = import(
            &mut store,
            host,
            &golden("F18", &format!("{} full", host.as_str())),
        );
        let expected = expected("F18")
            .into_iter()
            .filter(|item| item["host"] == host.as_str())
            .collect::<Vec<_>>();
        assert_eq!(results.len(), expected.len(), "{host:?}: {results:?}");
        for (result, want) in results.iter().zip(&expected) {
            let native = want["native_session_id"].as_str().unwrap();
            assert_eq!(result.native_session_id.as_deref(), Some(native));
            assert_eq!(
                result.outcome,
                SessionOutcome::Imported {
                    records_new: want["records"].as_u64().unwrap() as usize,
                    records_enriched: 0
                },
                "{native}"
            );
            let id = format!("{}-{native}", host.as_str());
            let session = store.session(&id).unwrap().expect("session row");
            assert_eq!(session.meta.host, host);
            assert_eq!(session.meta.source, SessionSource::ReadersCli);
            assert_eq!(session.meta.native_session_id.as_deref(), Some(native));
            assert_eq!(
                session.meta.surface,
                want["source_surface"].as_str().map(str::to_owned)
            );
            assert_eq!(session.meta.cwd, want["cwd"].as_str().map(str::to_owned));
            assert_eq!(
                session.meta.git_branch,
                want["git_branch"].as_str().map(str::to_owned)
            );
            assert_eq!(
                session.meta.title, None,
                "reader titles derive from prompts and are not kept"
            );
            let started = want["started_at"]
                .as_str()
                .map(|value| xt_store::timestamp::parse(value).unwrap().1);
            assert_eq!(session.meta.started_at_ms, started);
            let records = store.records(&id).unwrap();
            assert_eq!(records.len() as u64, want["records"].as_u64().unwrap());
            assert!(records.iter().all(|r| r.content_json.is_none()));
            assert!(
                records
                    .iter()
                    .all(|r| r.tool_uses.iter().all(|t| t.input_json.is_none()))
            );
            assert!(
                records
                    .iter()
                    .all(|r| r.ts.is_some() || host == Host::Cursor),
                "timestamps are preserved"
            );
            let cursor = store
                .source_cursor(
                    SessionSource::ReadersCli,
                    &format!("{}:{}", host.as_str(), result.path.as_deref().unwrap()),
                )
                .unwrap()
                .expect("locator cursor");
            assert!(cursor.position > 0);
        }
    }
    // Usage: the Cursor store carries readable usage on its tool call, the
    // transcript carries none, and Codex carries the session's cumulative usage.
    let cursor_store = store
        .records("cursor-00000000-0000-4000-8000-000000000183")
        .unwrap();
    let with_usage = cursor_store
        .iter()
        .filter(|r| r.usage.is_some())
        .collect::<Vec<_>>();
    assert_eq!(with_usage.len(), 1);
    assert_eq!(with_usage[0].usage.as_ref().unwrap().input_tokens, Some(50));
    assert_eq!(
        with_usage[0].usage.as_ref().unwrap().output_tokens,
        Some(12)
    );
    assert!(with_usage[0].model.as_deref() == Some("fixture-model"));
    let transcript = store
        .records("cursor-00000000-0000-4000-8000-000000000182")
        .unwrap();
    assert!(
        transcript.iter().all(|r| r.usage.is_none()),
        "missing usage stays unknown, never zero"
    );
    assert!(transcript.iter().any(|r| r.tool_use_count == Some(1)));
    let codex = store
        .records("codex-00000000-0000-4000-8000-000000000181")
        .unwrap();
    assert!(
        codex
            .iter()
            .any(|r| r.model.as_deref() == Some("gpt-5.3-codex"))
    );
    assert!(codex.iter().any(|r| r.tool_use_count == Some(1)));

    // Repeated import changes nothing.
    let before = store.counts().unwrap();
    for host in [Host::Codex, Host::Cursor] {
        for result in import(
            &mut store,
            host,
            &golden("F18", &format!("{} full", host.as_str())),
        ) {
            assert_eq!(
                result.outcome,
                SessionOutcome::Imported {
                    records_new: 0,
                    records_enriched: 0
                },
                "{result:?}"
            );
        }
    }
    assert_eq!(store.counts().unwrap(), before);
}

#[test]
fn metadata_only_streams_discover_sessions_without_writing_rows() {
    let mut store = Store::open_in_memory().unwrap();
    let results = import(
        &mut store,
        Host::Cursor,
        &golden("F20", "cursor metadata-only"),
    );
    assert_eq!(results.len(), 3);
    for result in &results {
        assert_eq!(
            result.outcome,
            SessionOutcome::Imported {
                records_new: 0,
                records_enriched: 0
            }
        );
        assert!(
            store
                .session(result.conversation_id.as_deref().unwrap())
                .unwrap()
                .is_none()
        );
    }
    let discovered = store.discovered_sessions(Host::Cursor).unwrap();
    assert_eq!(discovered.len(), 3);
    let recorded = discovered
        .iter()
        .find(|d| d.native_session_id.ends_with("204"))
        .unwrap();
    assert_eq!(recorded.surface.as_deref(), Some("cursor-cli"));
    assert!(recorded.started_at_ms.is_some());
    let legacy = discovered
        .iter()
        .find(|d| d.native_session_id.ends_with("203"))
        .unwrap();
    assert_eq!(
        legacy.surface, None,
        "a legacy store without a surface stays unknown"
    );
    assert_eq!(store.counts().unwrap().sessions, 0);
}

#[test]
fn malformed_sessions_are_skipped_explicitly_while_the_rest_import() {
    let mut store = Store::open_in_memory().unwrap();
    let mut lines = golden("F18", "cursor full");
    // Session 1 (the store): rename a header field -> contract change.
    lines[0] = rekey(&lines[0], "native_session_id", "native_session");
    let results = import(&mut store, Host::Cursor, &lines);
    assert_eq!(results.len(), 2);
    match &results[0].outcome {
        SessionOutcome::Skipped { reason } => assert!(reason.contains("contract"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(
            results[1].outcome,
            SessionOutcome::Imported { records_new: 7, .. }
        ),
        "{results:?}"
    );
    assert_eq!(store.counts().unwrap().sessions, 1);

    // A record that violates the canonical shape skips its session only.
    let mut lines = golden("F18", "cursor full");
    let second_header = lines.iter().rposition(|l| is_header(l)).unwrap();
    lines[second_header + 1] = r#"{"type":"assistant","uuid":5,"message":{}}"#.into();
    let mut store = Store::open_in_memory().unwrap();
    let results = import(&mut store, Host::Cursor, &lines);
    assert!(matches!(
        results[0].outcome,
        SessionOutcome::Imported { records_new: 5, .. }
    ));
    match &results[1].outcome {
        SessionOutcome::Skipped { reason } => assert!(reason.contains("line"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert!(
        store
            .session("cursor-00000000-0000-4000-8000-000000000182")
            .unwrap()
            .is_none()
    );

    // A header for another host, and a stream without any header at all.
    let mut lines = golden("F18", "codex full");
    lines[0] = reset(&lines[0], "host", "cursor");
    let blocks = parse_stream(&lines, Host::Codex, SessionSource::ReadersCli).unwrap();
    assert!(
        matches!(blocks[0], BlockOutcome::Malformed { reason, .. } if reason.contains("another host"))
    );
    let headless = golden("F18", "codex full")[1..].to_vec();
    assert_eq!(
        parse_stream(&headless, Host::Codex, SessionSource::ReadersCli).unwrap_err(),
        StreamError::RecordBeforeHeader { line: 1 }
    );
}

fn reader_home(root: &Path, host: Host) -> PathBuf {
    let home = root.join(format!("home-{}", host.as_str()));
    let sources = match host {
        Host::Codex => home.join(".codex/sessions"),
        _ => home.join(".cursor/chats"),
    };
    fs::create_dir_all(sources).unwrap();
    home
}

#[test]
fn reader_hosts_report_missing_sources_runtime_and_pin_explicitly() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut store = Store::open_in_memory().unwrap();
    let pin = repo().join(".plugin-pin");
    let empty = temp.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let report = import_native(
        &mut store,
        &ImportRequest {
            home: &empty,
            hosts: &[Host::Codex, Host::Cursor],
            pin: &pin,
            plugin_root: None,
            python: None,
            observed_at: 1,
        },
    );
    assert!(
        report
            .hosts
            .iter()
            .all(|h| h.status == HostStatus::MissingSource),
        "{report:?}"
    );

    let home = reader_home(temp.path(), Host::Codex);
    let missing = temp.path().join("nonexistent-python");
    let report = import_native(
        &mut store,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Codex],
            pin: &pin,
            plugin_root: None,
            python: Some(missing.as_os_str()),
            observed_at: 1,
        },
    );
    assert_eq!(
        report.hosts[0].status,
        HostStatus::MissingRuntime,
        "{report:?}"
    );

    // Python present, but no pinned producer supplied at all.
    let report = import_native(
        &mut store,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Codex],
            pin: &pin,
            plugin_root: None,
            python: None,
            observed_at: 1,
        },
    );
    assert_eq!(
        report.hosts[0].status,
        HostStatus::PinMismatch,
        "{report:?}"
    );
    assert!(
        report.hosts[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("no pinned plugin root")
    );

    // A checkout at some other commit is not the pinned producer.
    let checkout = temp.path().join("checkout");
    fs::create_dir_all(checkout.join("plugins/memhub/scripts")).unwrap();
    fs::write(
        checkout.join("plugins/memhub/scripts/readers_cli.py"),
        "print('not the pinned reader')\n",
    )
    .unwrap();
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
            .output()
            .unwrap();
        assert!(status.status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "synthetic"]);
    let report = import_native(
        &mut store,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Codex],
            pin: &pin,
            plugin_root: Some(&checkout.join("plugins/memhub")),
            python: None,
            observed_at: 1,
        },
    );
    assert_eq!(
        report.hosts[0].status,
        HostStatus::PinMismatch,
        "{report:?}"
    );
    assert!(
        report.hosts[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("pinned commit")
    );
    assert_eq!(
        store.counts().unwrap().sessions,
        0,
        "nothing is imported from an unverified producer"
    );
    assert!(!report.complete());
}

#[test]
fn rejected_and_uuid_less_records_make_coverage_partial_not_complete() {
    let mut store = Store::open_in_memory().unwrap();
    let codex = golden("F18", "codex full");
    assert!(matches!(
        import(&mut store, Host::Codex, &codex)[0].outcome,
        SessionOutcome::Imported { records_new: 5, .. }
    ));
    // A second session that reuses a UUID owned by the first one, plus a record without a UUID.
    let mut stream = golden("F18", "cursor full");
    let taken = codex.iter().find(|line| !is_header(line)).unwrap().clone();
    let header_index = stream.iter().rposition(|line| is_header(line)).unwrap();
    stream.insert(header_index + 1, taken);
    stream.insert(
        header_index + 2,
        r#"{"type":"assistant","message":{"role":"assistant","content":[]}}"#.into(),
    );
    let results = import(&mut store, Host::Cursor, &stream);
    assert!(
        matches!(
            results[0].outcome,
            SessionOutcome::Imported { records_new: 5, .. }
        ),
        "{results:?}"
    );
    match &results[1].outcome {
        SessionOutcome::Partial {
            records_new,
            records_dropped,
            rejections,
            ..
        } => {
            assert_eq!(*records_new, 7, "the session's own records still import");
            assert_eq!(*records_dropped, 2);
            assert!(
                rejections.contains(&"rejected_ownership".to_owned())
                    && rejections.contains(&"missing_uuid".to_owned()),
                "{rejections:?}"
            );
        }
        other => panic!("expected partial coverage, got {other:?}"),
    }
    let report = xt_ingest::native::ImportReport {
        hosts: vec![xt_ingest::native::HostReport {
            host: Host::Cursor,
            status: HostStatus::Incomplete,
            detail: None,
            diagnostics: Vec::new(),
            sessions: results,
        }],
    };
    assert!(
        !report.complete(),
        "a partial session can never read as complete coverage"
    );
}

fn stream_of(lines: Vec<String>) -> impl Iterator<Item = std::io::Result<String>> {
    lines.into_iter().map(Ok)
}

fn synthetic_session(native: &str, records: usize) -> Vec<String> {
    let mut lines = vec![serde_json::json!({
        "type": "session", "host": "codex", "native_session_id": native,
        "conversation_id": format!("codex-{native}"), "source_surface": "codex_cli",
        "started_at": "2026-09-07T12:00:00.000Z", "cwd": "/repo/fixture", "git_branch": null,
        "title": null, "path": format!("$HOME/.codex/sessions/{native}.jsonl"), "mtime": 1788782400.0
    })
    .to_string()];
    for index in 0..records {
        let role = if index.is_multiple_of(2) {
            "user"
        } else {
            "assistant"
        };
        lines.push(serde_json::json!({
            "uuid": format!("5555{}-5555-4555-8555-{index:012}", &native[native.len() - 4..]),
            "type": role, "cwd": "/repo/fixture",
            "timestamp": format!("2026-09-07T12:{:02}:{:02}Z", (index / 60) % 60, index % 60),
            "message": {"role": role, "content": [{"type": "text", "text": format!("turn {index}")}]}
        })
        .to_string());
    }
    lines
}

#[test]
fn streaming_import_drains_batches_and_holds_the_trailing_session_until_a_confirmed_exit() {
    use xt_ingest::native::readers_cli::{ReaderError, ReaderOutcome};
    use xt_ingest::native::{HostStatus, import_reader_lines};
    let big = "00000000-0000-4000-8000-00000000b16e";
    let exact = "00000000-0000-4000-8000-00000000e2ac";
    let complete = || {
        Ok(ReaderOutcome {
            diagnostics: Vec::new(),
            complete: true,
        })
    };

    // A 4,500-record session drains in bounded batches; a session of exactly
    // 2,000 records still records its cursor from an empty final batch.
    let mut store = Store::open_in_memory().unwrap();
    let mut lines = synthetic_session(big, 4_500);
    lines.extend(synthetic_session(exact, 2_000));
    let report = import_reader_lines(
        &mut store,
        Host::Codex,
        "test".into(),
        stream_of(lines),
        1,
        complete,
    );
    assert_eq!(report.status, HostStatus::Complete, "{report:?}");
    assert_eq!(
        report.sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 4_500,
            records_enriched: 0
        }
    );
    assert_eq!(
        report.sessions[1].outcome,
        SessionOutcome::Imported {
            records_new: 2_000,
            records_enriched: 0
        }
    );
    for native in [big, exact] {
        let key = format!("codex:$HOME/.codex/sessions/{native}.jsonl");
        assert!(
            store
                .source_cursor(SessionSource::ReadersCli, &key)
                .unwrap()
                .is_some(),
            "{native} keeps its cursor"
        );
    }
    assert_eq!(
        store.records(&format!("codex-{exact}")).unwrap().len(),
        2_000
    );

    // The producer dies after emitting part of a session: the earlier session
    // is complete, the trailing one keeps its committed batches but is reported
    // as ended early and gets no cursor.
    let mut store = Store::open_in_memory().unwrap();
    let mut lines = synthetic_session(exact, 3);
    lines.extend(synthetic_session(big, 2_300));
    let failed = || Err(ReaderError::Failed("reader exited with status 1".into()));
    let report = import_reader_lines(
        &mut store,
        Host::Codex,
        "test".into(),
        stream_of(lines),
        1,
        failed,
    );
    assert_eq!(report.status, HostStatus::ReaderFailed, "{report:?}");
    assert_eq!(
        report.sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 3,
            records_enriched: 0
        }
    );
    match &report.sessions[1].outcome {
        SessionOutcome::Skipped { reason } => assert!(
            reason.contains("ended before completing") && reason.contains("1 earlier batches"),
            "{reason}"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        store.records(&format!("codex-{big}")).unwrap().len(),
        2_000,
        "the committed batch stays"
    );
    assert!(
        store
            .source_cursor(
                SessionSource::ReadersCli,
                &format!("codex:$HOME/.codex/sessions/{big}.jsonl")
            )
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .source_cursor(
                SessionSource::ReadersCli,
                &format!("codex:$HOME/.codex/sessions/{exact}.jsonl")
            )
            .unwrap()
            .is_some()
    );

    // A read error mid-stream is a reader failure with the same trailing rule.
    let mut store = Store::open_in_memory().unwrap();
    let mut lines: Vec<std::io::Result<String>> =
        synthetic_session(exact, 2).into_iter().map(Ok).collect();
    lines.push(Err(std::io::Error::other("pipe broke")));
    let report = import_reader_lines(&mut store, Host::Codex, "test".into(), lines, 1, complete);
    assert_eq!(report.status, HostStatus::ReaderFailed);
    assert!(
        matches!(&report.sessions[0].outcome, SessionOutcome::Skipped { reason } if reason.contains("0 earlier batches"))
    );
    assert!(!report.sessions.is_empty());
}

#[test]
fn a_malformed_successor_header_completes_the_previous_session() {
    use xt_ingest::native::readers_cli::ReaderOutcome;
    use xt_ingest::native::{HostStatus, import_reader_lines};
    let first = "00000000-0000-4000-8000-00000000f157";
    let mut store = Store::open_in_memory().unwrap();
    let mut lines = synthetic_session(first, 3);
    lines.push(r#"{"type":"session","host":"codex","native_session":"broken"}"#.into());
    lines.push(synthetic_session(first, 1).pop().unwrap()); // swallowed by the malformed session
    let report = import_reader_lines(
        &mut store,
        Host::Codex,
        "test".into(),
        stream_of(lines),
        1,
        || {
            Ok(ReaderOutcome {
                diagnostics: Vec::new(),
                complete: true,
            })
        },
    );
    assert_eq!(report.status, HostStatus::Incomplete, "{report:?}");
    assert_eq!(report.sessions.len(), 2);
    assert_eq!(
        report.sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 3,
            records_enriched: 0
        }
    );
    assert!(
        store
            .source_cursor(
                SessionSource::ReadersCli,
                &format!("codex:$HOME/.codex/sessions/{first}.jsonl")
            )
            .unwrap()
            .is_some(),
        "the predecessor keeps its cursor"
    );
    assert!(
        matches!(&report.sessions[1].outcome, SessionOutcome::Skipped { reason } if reason.contains("contract") && reason.contains("line 5")),
        "{:?}",
        report.sessions[1]
    );
    assert_eq!(report.sessions[1].native_session_id, None);
    // The whole-stream helper agrees: the predecessor is ready, the successor malformed.
    let blocks = parse_stream(
        synthetic_session(first, 3)
            .into_iter()
            .chain([r#"{"type":"session","host":"codex","native_session":"broken"}"#.to_owned()])
            .collect::<Vec<_>>(),
        Host::Codex,
        SessionSource::ReadersCli,
    )
    .unwrap();
    assert!(matches!(blocks[0], BlockOutcome::Ready(ref ready) if ready.records.len() == 3));
    assert!(matches!(
        blocks[1],
        BlockOutcome::Malformed {
            native_session_id: None,
            line: 5,
            ..
        }
    ));
}

#[cfg(unix)]
#[test]
fn an_explicit_relative_interpreter_path_is_anchored_before_the_reader_changes_directory() {
    use std::os::unix::fs::PermissionsExt;
    use xt_ingest::native::readers_cli::resolve_python;
    let temp = tempfile::TempDir::new().unwrap();
    let wrapper = temp.path().join("bin").join("python-wrapper");
    fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
    fs::write(&wrapper, "#!/bin/sh\nexec python3 \"$@\"\n").unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    // A relative spelling of the wrapper from the current directory.
    let cwd = std::env::current_dir().unwrap();
    let ups = "../".repeat(cwd.components().count() - 1);
    let relative = format!(
        "{ups}{}",
        wrapper
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .trim_start_matches('/')
    );
    assert!(Path::new(&relative).is_relative());
    let resolved = resolve_python(Some(std::ffi::OsStr::new(&relative))).unwrap();
    let resolved = Path::new(&resolved);
    assert!(resolved.is_absolute(), "{resolved:?}");
    assert_eq!(
        resolved.canonicalize().unwrap(),
        wrapper.canonicalize().unwrap()
    );
    // A bare command name stays a PATH lookup.
    assert_eq!(
        resolve_python(Some(std::ffi::OsStr::new("python3"))).unwrap(),
        "python3"
    );
}
