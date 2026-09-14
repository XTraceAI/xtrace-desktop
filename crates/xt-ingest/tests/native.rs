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
    HostStatus, ImportRequest, SessionOutcome, import_native, import_reader_lines,
    readers_cli::ReaderOutcome,
};
use xt_store::{Host, SessionSource, Store};

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
    import_reader_lines(
        store,
        host,
        "fixture".into(),
        lines.iter().cloned().map(Ok),
        1_788_782_400_000,
        || {
            Ok(ReaderOutcome {
                diagnostics: vec![],
                complete: true,
            })
        },
    )
    .sessions
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
    let results = import(&mut store, Host::Codex, &lines);
    assert!(
        matches!(&results[0].outcome, SessionOutcome::Skipped { reason } if reason.contains("another host"))
    );
    let report = import_reader_lines(
        &mut store,
        Host::Codex,
        "fixture".into(),
        golden("F18", "codex full").into_iter().skip(1).map(Ok),
        1,
        || {
            Ok(ReaderOutcome {
                diagnostics: vec![],
                complete: true,
            })
        },
    );
    assert_eq!(report.status, HostStatus::ReaderFailed);
    assert!(
        report
            .detail
            .unwrap()
            .contains("precedes any session header")
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
    // A partial session records no cursor: the position would claim the
    // source was consumed past records that were not imported. A complete
    // session keeps its cursor.
    let path_of = |line: &str| -> String {
        serde_json::from_str::<Value>(line).unwrap()["path"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert!(
        store
            .source_cursor(
                SessionSource::ReadersCli,
                &format!("cursor:{}", path_of(&stream[header_index]))
            )
            .unwrap()
            .is_none(),
        "no cursor for a partial session"
    );
    assert!(
        store
            .source_cursor(
                SessionSource::ReadersCli,
                &format!("codex:{}", path_of(&codex[0]))
            )
            .unwrap()
            .is_some(),
        "a complete session keeps its cursor"
    );
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

#[test]
fn invalid_json_withholds_cursor_and_stream_errors_drain_the_producer() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use xt_ingest::native::readers_cli::ReaderOutcome;
    use xt_ingest::native::{HostStatus, import_reader_lines};
    let complete = || {
        Ok(ReaderOutcome {
            diagnostics: Vec::new(),
            complete: true,
        })
    };
    let first = "00000000-0000-4000-8000-00000000f157";
    let second = "00000000-0000-4000-8000-000000005ecd";
    // Truncated JSON cannot prove the predecessor complete. Withhold its
    // cursor, discard the pending batch and ignore orphan successor records.
    for successor in [
        r#"{"type":"session","host":"codex","native_session_id":"x","mtime":1.0,"path":"p""#
            .to_owned(),
        r#"{"type":"session""#.to_owned(),
        r#"{ "type" : "session", "host": "codex"#.to_owned(),
    ] {
        let mut store = Store::open_in_memory().unwrap();
        let mut lines = synthetic_session(first, 3);
        lines.push(successor);
        lines.extend(synthetic_session(second, 4).into_iter().skip(1));
        lines.extend(synthetic_session(second, 4));
        let report = import_reader_lines(
            &mut store,
            Host::Codex,
            "test".into(),
            stream_of(lines),
            1,
            complete,
        );
        assert_eq!(report.status, HostStatus::Incomplete, "{report:?}");
        assert_eq!(report.sessions.len(), 2);
        assert!(
            matches!(&report.sessions[0].outcome, SessionOutcome::Skipped { reason } if reason.contains("line 5"))
        );
        assert!(
            store
                .source_cursor(
                    SessionSource::ReadersCli,
                    &format!("codex:$HOME/.codex/sessions/{first}.jsonl")
                )
                .unwrap()
                .is_none()
        );
        assert_eq!(store.counts().unwrap().sessions, 1);
        // Recovery occurs within the same stream after a decoded header.
        assert!(matches!(
            report.sessions[1].outcome,
            SessionOutcome::Imported { records_new: 4, .. }
        ));
    }

    // A record before any header is a stream error; the remaining output is
    // still drained to its end before the producer is awaited.
    let mut store = Store::open_in_memory().unwrap();
    let consumed = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&consumed);
    let mut lines = synthetic_session(first, 50);
    lines.remove(0);
    let total = lines.len();
    let counting = lines.into_iter().map(move |line| {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(line)
    });
    let seen = Arc::clone(&consumed);
    let report = import_reader_lines(
        &mut store,
        Host::Codex,
        "test".into(),
        counting,
        1,
        move || {
            assert_eq!(
                seen.load(Ordering::SeqCst),
                total,
                "finish runs only after the stream is drained"
            );
            Ok(ReaderOutcome {
                diagnostics: Vec::new(),
                complete: true,
            })
        },
    );
    assert_eq!(report.status, HostStatus::ReaderFailed);
    assert!(
        report
            .detail
            .as_deref()
            .unwrap()
            .contains("precedes any session header")
    );
    assert_eq!(consumed.load(Ordering::SeqCst), total);
}

#[test]
fn header_only_streams_leave_no_canonical_row_or_cursor_in_the_streaming_path() {
    use xt_ingest::native::readers_cli::ReaderOutcome;
    use xt_ingest::native::{HostStatus, import_reader_lines};
    let mut store = Store::open_in_memory().unwrap();
    let mut lines = golden("F20", "cursor metadata-only");
    // A dropped-only session (a record without a uuid) is partial, not a row.
    let dropped = "00000000-0000-4000-8000-0000000000d1";
    lines.extend(synthetic_session(dropped, 1).into_iter().map(|line| {
        let mut value: Value = serde_json::from_str(&line).unwrap();
        let object = value.as_object_mut().unwrap();
        if is_header(&line) {
            object.insert("host".into(), "cursor".into());
            object.insert("conversation_id".into(), format!("cursor-{dropped}").into());
            object.insert("source_surface".into(), "cursor-cli".into());
            object.insert(
                "path".into(),
                format!("$HOME/.cursor/chats/x/{dropped}/store.db").into(),
            );
        } else {
            object.remove("uuid");
        }
        value.to_string()
    }));
    let report = import_reader_lines(
        &mut store,
        Host::Cursor,
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
    assert_eq!(report.sessions.len(), 4);
    for session in &report.sessions[..3] {
        assert_eq!(
            session.outcome,
            SessionOutcome::Imported {
                records_new: 0,
                records_enriched: 0
            }
        );
    }
    assert!(
        matches!(
            &report.sessions[3].outcome,
            SessionOutcome::Partial {
                records_dropped: 1,
                ..
            }
        ),
        "{:?}",
        report.sessions[3]
    );
    assert_eq!(
        store.counts().unwrap().sessions,
        0,
        "no canonical row for a session without a storable record"
    );
    assert_eq!(store.discovered_sessions(Host::Cursor).unwrap().len(), 4);
    for session in &report.sessions {
        let key = format!("cursor:{}", session.path.as_deref().unwrap());
        assert!(
            store
                .source_cursor(SessionSource::ReadersCli, &key)
                .unwrap()
                .is_none(),
            "{key} must not record a cursor"
        );
    }
}

#[test]
fn headers_with_empty_labels_are_malformed_before_any_record_arrives() {
    // The writer rejects an empty identity label once a record exists; the
    // header check rejects it first, so validity never depends on record count.
    for (key, value, needle) in [
        ("source_surface", "", "empty label"),
        ("cwd", " ", "empty label"),
        ("git_branch", "", "empty label"),
        ("path", "", "source path"),
    ] {
        let mut lines = golden("F18", "cursor full");
        let mut header: Value = serde_json::from_str(&lines[0]).unwrap();
        header
            .as_object_mut()
            .unwrap()
            .insert(key.into(), Value::String(value.into()));
        lines[0] = header.to_string();
        let mut store = Store::open_in_memory().unwrap();
        let results = import(&mut store, Host::Cursor, &lines);
        assert_eq!(results.len(), 2, "{key}: {results:?}");
        match &results[0].outcome {
            SessionOutcome::Skipped { reason } => {
                assert!(reason.contains(needle), "{key}: {reason}");
            }
            other => panic!("{key}: {other:?}"),
        }
        assert!(matches!(
            results[1].outcome,
            SessionOutcome::Imported { records_new: 7, .. }
        ));
        assert_eq!(store.counts().unwrap().sessions, 1, "{key}");
        assert_eq!(
            store.discovered_sessions(Host::Cursor).unwrap().len(),
            1,
            "{key}: a malformed header registers nothing"
        );
    }
}

#[test]
fn nested_session_discriminators_in_truncated_records_are_not_boundaries() {
    use xt_ingest::native::readers_cli::ReaderOutcome;
    use xt_ingest::native::{HostStatus, import_reader_lines};
    let first = "00000000-0000-4000-8000-00000000ae57";
    // A record cut off inside its tool input, which happens to hold the
    // discriminator or the header-only keys: a malformed record of the
    // active session, not a successor header, so the session is abandoned
    // rather than completed.
    for truncated in [
        r#"{"uuid":"5555ae57-5555-4555-8555-000000000099","type":"assistant","cwd":"/repo/fixture","timestamp":"2026-09-07T12:00:09Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"type":"session","command":"ls"#,
        r#"{"uuid":"5555ae57-5555-4555-8555-000000000099","type":"assistant","cwd":"/repo/fixture","timestamp":"2026-09-07T12:00:09Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Write","input":{"mtime":1.0,"native_session_id":"x","content":"..."#,
    ] {
        let mut store = Store::open_in_memory().unwrap();
        let mut lines = synthetic_session(first, 3);
        lines.push(truncated.to_owned());
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
        assert_eq!(report.sessions.len(), 1, "no phantom successor: {report:?}");
        match &report.sessions[0].outcome {
            SessionOutcome::Skipped { reason } => assert!(
                reason.contains("line 5") && reason.contains("0 earlier batches"),
                "{reason}"
            ),
            other => panic!("{other:?}"),
        }
        assert!(
            store
                .source_cursor(
                    SessionSource::ReadersCli,
                    &format!("codex:$HOME/.codex/sessions/{first}.jsonl")
                )
                .unwrap()
                .is_none(),
            "an abandoned session commits no cursor"
        );
        assert_eq!(store.counts().unwrap().sessions, 0);
    }
}

#[test]
fn reader_non_records_reject_coverage_and_resume_at_the_next_header() {
    for invalid in [
        r#"{}"#,
        r#"{"type":"assistnt"}"#,
        r#"{"type":"system"}"#,
        r#"{"type":"attachment"}"#,
    ] {
        let mut store = Store::open_in_memory().unwrap();
        let first = "00000000-0000-4000-8000-00000000f157";
        let second = "00000000-0000-4000-8000-000000005ecd";
        let mut lines = synthetic_session(first, 3);
        lines.push(invalid.into());
        lines.extend(synthetic_session(second, 4));
        let results = import(&mut store, Host::Codex, &lines);
        assert!(
            matches!(&results[0].outcome, SessionOutcome::Skipped { .. }),
            "{invalid}: {results:?}"
        );
        assert!(
            store
                .source_cursor(
                    SessionSource::ReadersCli,
                    &format!("codex:$HOME/.codex/sessions/{first}.jsonl")
                )
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            results[1].outcome,
            SessionOutcome::Imported { records_new: 4, .. }
        ));
    }
}

#[test]
fn pinned_checkout_exports_only_committed_python_modules() {
    use xt_ingest::native::readers_cli::{Pin, verify_pin};
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path();
    let plugin = root.join("plugins/memhub");
    fs::create_dir_all(plugin.join("scripts")).unwrap();
    fs::write(plugin.join("scripts/readers_cli.py"), "import json\n").unwrap();
    fs::write(root.join(".gitignore"), "json.py\n").unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "Synthetic reader"]);
    let pin = Pin {
        repository: "fixture".into(),
        commit: git(&["rev-parse", "HEAD"]),
        plugin_root: "plugins/memhub".into(),
        plugin_version: "0.0.0".into(),
        reader_sources: [(
            "plugins/memhub/scripts".into(),
            git(&["rev-parse", "HEAD:plugins/memhub/scripts"]),
        )]
        .into_iter()
        .collect(),
    };
    assert!(verify_pin(&pin, &plugin).is_ok());
    fs::write(
        plugin.join("scripts/json.py"),
        "raise RuntimeError('unverified module')\n",
    )
    .unwrap();
    assert!(git(&["status", "--porcelain"]).is_empty());
    let producer = verify_pin(&pin, &plugin).unwrap();
    assert_eq!(
        fs::read_to_string(&producer.script).unwrap(),
        "import json\n"
    );
    assert!(!producer.script.parent().unwrap().join("json.py").exists());
    assert!(!producer.script.starts_with(&plugin));
}

#[cfg(unix)]
#[test]
fn inaccessible_reader_roots_are_not_reported_missing() {
    use std::os::unix::fs::PermissionsExt;
    for (host, directory) in [(Host::Codex, ".codex"), (Host::Cursor, ".cursor")] {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("home");
        let root = home.join(directory);
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let report = import_native(
            &mut store,
            &ImportRequest {
                home: &home,
                hosts: &[host],
                pin: &repo().join(".plugin-pin"),
                plugin_root: None,
                python: None,
                observed_at: 1,
            },
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(report.hosts[0].status, HostStatus::ReaderFailed);
        assert_eq!(store.counts().unwrap().records, 0);
    }
}
