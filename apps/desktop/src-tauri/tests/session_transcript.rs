//! Opening one session's transcript through the app's own command path.
//!
//! The suite proves the seam the view depends on: a canonical identifier is
//! resolved **against the index** into a host, a native identity and the
//! locators that session was measured through, read under the home the app was
//! started with, and returned as records the view can render. What the view
//! sends is an identifier and nothing else — there is no path in the request,
//! so there is no path for it to choose.
//!
//! Every transcript here is written by the test: short synthetic lines in the
//! canonical shape, never a real conversation.
#![cfg(unix)]
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use xt_ingest::native::{
    ImportRequest, ProducerSource, import_native,
    readers_cli::{CancelToken, read_pin},
};
use xt_store::{Host, SessionMeta, SessionSource, Store, StoreCounts};
use xtrace_desktop::{
    dto::{
        ReaderLimit, ReaderUnavailableCause, SessionGeneration, SessionSourceReason,
        SessionSourceStatus,
    },
    native_index::DetailReaders,
    state::{AppState, StartupOptions},
    transcript_dto::{RecordRole, SourceBlock, SourcePayload, SourceRecord, SourceResultPart},
};

const OBSERVED_AT: i64 = 1_788_782_400_000;
const SESSION: &str = "00000000-0000-4000-8000-00000000aaaa";
const PROJECT: &str = "-repo-fixture";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn uuid(index: usize) -> String {
    format!("11111111-1111-4111-8111-{index:012}")
}

/// One synthetic exchange: a question, an answer that calls a tool, and the
/// record the host writes when that call returns.
fn transcript(session: &str) -> String {
    [
        json!({
            "uuid": uuid(0), "type": "user", "sessionId": session, "cwd": "/repo/fixture",
            "timestamp": "2026-09-07T12:00:00.000Z",
            "message": {"role": "user", "content": [
                {"type": "text", "text": "what does a.txt say?"}]}
        }),
        json!({
            "uuid": uuid(1), "type": "assistant", "sessionId": session, "cwd": "/repo/fixture",
            "timestamp": "2026-09-07T12:00:01.000Z",
            "message": {"role": "assistant", "model": "claude-fixture", "content": [
                {"type": "thinking", "thinking": "read it first", "signature": "sig"},
                {"type": "text", "text": "Reading it."},
                {"type": "tool_use", "id": "toolu_0001", "name": "Read",
                 "input": {"file_path": "/repo/fixture/a.txt"}}]}
        }),
        json!({
            "uuid": uuid(2), "type": "user", "sessionId": session, "cwd": "/repo/fixture",
            "timestamp": "2026-09-07T12:00:02.000Z",
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_0001",
                 "content": [{"type": "text", "text": "a.txt line one"},
                             {"type": "image", "source": {"data": "SECRETBYTES"}}]}]}
        }),
    ]
    .iter()
    .map(|line| format!("{line}\n"))
    .collect()
}

/// A home holding one Claude project with one session's transcript.
fn home_with(root: &Path, session: &str, body: &str) -> PathBuf {
    let home = root.join("home");
    let directory = home.join(".claude/projects").join(PROJECT);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join(format!("{session}.jsonl")), body).unwrap();
    home
}

fn source_path(home: &Path, session: &str) -> PathBuf {
    home.join(".claude/projects")
        .join(PROJECT)
        .join(format!("{session}.jsonl"))
}

/// Index the home into the database the app will open, exactly as the app
/// does, so every read below is judged against measurements really taken from
/// these bytes.
fn index(db: &Path, home: &Path) {
    fs::create_dir_all(db.parent().unwrap()).unwrap();
    let mut store = Store::open(db).unwrap();
    let report = import_native(
        &mut store,
        &ImportRequest {
            home,
            hosts: &[Host::Claude],
            producer: &ProducerSource::Bundle {
                pin: read_pin(&repo().join(".plugin-pin")).unwrap(),
                root: repo().join("vendor/agent-plugins"),
            },
            python: None,
            observed_at: OBSERVED_AT,
            cancel: None,
        },
    );
    assert!(report.complete(), "{report:?}");
}

fn state(root: &Path) -> AppState {
    AppState::build(
        StartupOptions {
            data_dir: Some(root.join("data")),
            native_home: Some(root.join("home")),
            ..Default::default()
        },
        || panic!("explicit data directory"),
        || panic!("explicit native home"),
    )
    .unwrap()
}

/// The canonical identifier the view would hold for the indexed session,
/// taken from the index rather than assumed: the view holds what the list
/// gave it, and this read has to resolve that back to a native identity.
fn canonical_id(db: &Path, native: &str) -> String {
    let store = Store::open(db).unwrap();
    let session = store
        .session(native)
        .unwrap()
        .unwrap_or_else(|| panic!("the index did not record {native}"));
    assert_eq!(session.meta.native_session_id.as_deref(), Some(native));
    assert_eq!(session.meta.host, Host::Claude);
    session.meta.session_id
}

fn counts(db: &Path) -> StoreCounts {
    Store::open(db).unwrap().counts().unwrap()
}

/// Open through the command path with no pinned reader: a Claude session
/// never consults one.
fn open(state: &AppState, id: &str) -> SessionSourceStatus {
    state
        .session_transcript(id, &CancelToken::new(), &DetailReaders::disabled())
        .unwrap()
}

fn records(status: SessionSourceStatus) -> Vec<SourceRecord> {
    match status {
        SessionSourceStatus::Loaded { records, .. } => records,
        SessionSourceStatus::Unavailable { reason } => panic!("expected records, got {reason:?}"),
    }
}

fn reason(status: SessionSourceStatus) -> SessionSourceReason {
    match status {
        SessionSourceStatus::Unavailable { reason } => reason,
        SessionSourceStatus::Loaded { records, .. } => {
            panic!("expected no source, got {} records", records.len())
        }
    }
}

#[test]
fn an_indexed_session_opens_its_own_records_exactly() {
    let root = tempfile::tempdir().unwrap();
    let home = home_with(root.path(), SESSION, &transcript(SESSION));
    let db = root.path().join("data/xtrace.db");
    index(&db, &home);
    let id = canonical_id(&db, SESSION);
    let state = state(root.path());
    let status = open(&state, &id);
    let SessionSourceStatus::Loaded {
        generation,
        sources,
        dropped_records,
        ref gaps,
        ..
    } = status
    else {
        panic!("{status:?}");
    };
    // The bytes that were measured are still the bytes that were read, the
    // whole session is one file, and nothing about it is unaccounted for.
    assert_eq!(generation, SessionGeneration::Indexed { appended: false });
    assert_eq!((sources, dropped_records), (1, 0));
    assert!(gaps.is_empty(), "{gaps:?}");

    let records = records(status);
    assert_eq!(
        records.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        [uuid(0), uuid(1), uuid(2)]
    );
    assert_eq!(
        records.iter().map(|r| r.role).collect::<Vec<_>>(),
        [RecordRole::User, RecordRole::Assistant, RecordRole::User]
    );
    assert_eq!(records[1].at.as_deref(), Some("2026-09-07T12:00:01.000Z"));
    // Each block, in the record's order, identified by where it sits in it.
    assert_eq!(
        records[1].blocks,
        [
            SourceBlock::Thinking {
                index: 0,
                text: "read it first".into(),
            },
            SourceBlock::Text {
                index: 1,
                text: "Reading it.".into(),
            },
            SourceBlock::ToolCall {
                index: 2,
                name: "Read".into(),
                call_id: Some("toolu_0001".into()),
                input: Some(SourcePayload::Json {
                    value: json!({"file_path": "/repo/fixture/a.txt"}),
                }),
            },
        ]
    );
    assert_eq!(
        records[2].blocks,
        [SourceBlock::ToolResult {
            index: 0,
            call_id: Some("toolu_0001".into()),
            failed: false,
            parts: vec![
                SourceResultPart::Text {
                    text: "a.txt line one".into()
                },
                SourceResultPart::Unshown {
                    label: Some("image".into())
                },
            ],
        }]
    );
    // The result's picture is named on the wire and never carried on it.
    let wire = serde_json::to_string(&records).unwrap();
    assert!(!wire.contains("SECRETBYTES"), "{wire}");
}

#[test]
fn opening_a_session_writes_nothing_anywhere() {
    let root = tempfile::tempdir().unwrap();
    let home = home_with(root.path(), SESSION, &transcript(SESSION));
    let db = root.path().join("data/xtrace.db");
    index(&db, &home);
    let id = canonical_id(&db, SESSION);
    let source = source_path(&home, SESSION);
    let before = (
        counts(&db),
        fs::read(&source).unwrap(),
        fs::metadata(&source).unwrap().modified().unwrap(),
    );
    let state = state(root.path());
    assert_eq!(records(open(&state, &id)).len(), 3);
    drop(state);
    let after = (
        counts(&db),
        fs::read(&source).unwrap(),
        fs::metadata(&source).unwrap().modified().unwrap(),
    );
    assert_eq!(before, after);
    // No copy of the transcript was left beside the database either.
    let stray: Vec<_> = fs::read_dir(root.path().join("data"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| !name.to_string_lossy().starts_with("xtrace.db"))
        .collect();
    assert!(stray.is_empty(), "{stray:?}");
}

#[test]
fn a_cancelled_read_returns_no_part_of_the_transcript() {
    let root = tempfile::tempdir().unwrap();
    let home = home_with(root.path(), SESSION, &transcript(SESSION));
    let db = root.path().join("data/xtrace.db");
    index(&db, &home);
    let id = canonical_id(&db, SESSION);
    let state = state(root.path());
    let cancel = CancelToken::new();
    cancel.cancel();
    let status = state
        .session_transcript(&id, &cancel, &DetailReaders::disabled())
        .unwrap();
    assert_eq!(reason(status), SessionSourceReason::Cancelled);
}

#[test]
fn a_session_whose_source_changed_under_the_index_is_refused_with_its_measurements_intact() {
    let root = tempfile::tempdir().unwrap();
    let home = home_with(root.path(), SESSION, &transcript(SESSION));
    let db = root.path().join("data/xtrace.db");
    index(&db, &home);
    let id = canonical_id(&db, SESSION);
    let measured = counts(&db);
    // The same length, different words: only the change time says so.
    let rewritten = transcript(SESSION).replace("a.txt line one", "b.txt line two");
    fs::write(source_path(&home, SESSION), &rewritten).unwrap();
    let state = state(root.path());
    assert_eq!(reason(open(&state, &id)), SessionSourceReason::Replaced);
    // The text is refused; what was measured about the session is untouched.
    assert_eq!(counts(&db), measured);
}

#[test]
fn a_session_this_index_cannot_name_a_source_for_says_so() {
    let root = tempfile::tempdir().unwrap();
    let home = home_with(root.path(), SESSION, &transcript(SESSION));
    let db = root.path().join("data/xtrace.db");
    index(&db, &home);
    {
        // A session the index holds without a native identity: measured, with
        // no local file this read could name.
        let mut store = Store::open(&db).unwrap();
        store
            .upsert_session(
                &SessionMeta::new("plugin-only", "claude", SessionSource::Plugin),
                false,
            )
            .unwrap();
    }
    let state = state(root.path());
    for id in ["plugin-only", "no-such-session", ""] {
        assert_eq!(
            reason(open(&state, id)),
            SessionSourceReason::NotIndexed,
            "{id}"
        );
    }
}

#[test]
fn another_host_without_a_reader_is_refused_by_name_rather_than_read() {
    // Codex and Cursor reach their sources only through the pinned reader.
    // With the index disabled there is none to give, and the refusal says
    // so; everything the index measured about them still stands.
    let root = tempfile::tempdir().unwrap();
    let home = home_with(root.path(), SESSION, &transcript(SESSION));
    let db = root.path().join("data/xtrace.db");
    index(&db, &home);
    {
        let mut store = Store::open(&db).unwrap();
        for (id, host) in [("codex-1", "codex"), ("cursor-1", "cursor")] {
            let mut meta = SessionMeta::new(id, host, SessionSource::ReadersCli);
            meta.native_session_id = Some(format!("native-{id}"));
            store.upsert_session(&meta, false).unwrap();
        }
    }
    let before = counts(&db);
    let state = state(root.path());
    for id in ["codex-1", "cursor-1"] {
        assert_eq!(
            reason(open(&state, id)),
            SessionSourceReason::ReaderUnavailable {
                cause: ReaderUnavailableCause::Index
            },
            "{id}"
        );
    }
    assert_eq!(counts(&db), before);
}

/// Fixture startup reads no local history at all, so there is no source to
/// open: the answer is that nothing here can name one, not a failure and not
/// an empty transcript.
#[cfg(all(debug_assertions, feature = "fixtures"))]
#[test]
fn fixture_startup_names_no_local_source_for_any_session() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(root.path().into()),
            fixture: Some("F1".into()),
            ..Default::default()
        },
        || panic!("fixture must not resolve the live directory"),
        || panic!("fixture must not resolve the home directory"),
    )
    .unwrap();
    assert_eq!(
        reason(open(&state, "any-session")),
        SessionSourceReason::NotIndexed
    );
    // Nor does it reach a reader, whatever one it is handed.
    let marker = root.path().join("started");
    let readers = DetailReaders::bundle(
        repo().join("vendor/agent-plugins"),
        Some(fake_python(root.path(), &marker, "exit 0").into()),
    );
    assert_eq!(
        reason(
            state
                .session_transcript("any-session", &CancelToken::new(), &readers)
                .unwrap()
        ),
        SessionSourceReason::NotIndexed
    );
    assert!(!marker.exists(), "fixture startup started a reader");
}

/// The blocks of one record, for a case that only cares about them.
fn blocks(state: &AppState, id: &str, index: usize) -> Vec<SourceBlock> {
    records(open(state, id))[index].blocks.clone()
}

#[test]
fn a_tool_name_and_a_payload_cross_exactly_as_the_record_wrote_them() {
    let root = tempfile::tempdir().unwrap();
    let odd = json!({
        "uuid": uuid(0), "type": "assistant", "sessionId": SESSION, "cwd": "/repo/fixture",
        "timestamp": "2026-09-07T12:00:00.000Z",
        "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "toolu_x", "name": "__proto__",
             "input": {"command": "echo '<script>x</script>\tünïcödé 🙂'"}}]}
    });
    let home = home_with(root.path(), SESSION, &format!("{odd}\n"));
    let db = root.path().join("data/xtrace.db");
    index(&db, &home);
    let id = canonical_id(&db, SESSION);
    let state = state(root.path());
    let Value::Object(input) = &odd["message"]["content"][0]["input"] else {
        unreachable!()
    };
    assert_eq!(
        blocks(&state, &id, 0),
        [SourceBlock::ToolCall {
            index: 0,
            name: "__proto__".into(),
            call_id: Some("toolu_x".into()),
            input: Some(SourcePayload::Json {
                value: Value::Object(input.clone()),
            }),
        }]
    );
}

#[test]
fn record_identities_that_differ_only_in_whitespace_stay_two_records() {
    // The parser trims only to decide whether an identity is usable, and the
    // store keys a record by the bytes it carried. The payload has to agree
    // with both, or a later jump from a measurement to a block compares an
    // identity against one it was never given.
    let root = tempfile::tempdir().unwrap();
    let bare = uuid(0);
    let padded = format!(" {bare}");
    let body: String = [&bare, &padded]
        .iter()
        .map(|id| {
            format!(
                "{}\n",
                json!({
                    "uuid": id, "type": "user", "sessionId": SESSION, "cwd": "/repo/fixture",
                    "timestamp": "2026-09-07T12:00:00.000Z",
                    "message": {"role": "user", "content": [{"type": "text", "text": "hi"}]}
                })
            )
        })
        .collect();
    let home = home_with(root.path(), SESSION, &body);
    let db = root.path().join("data/xtrace.db");
    index(&db, &home);
    let id = canonical_id(&db, SESSION);

    // Two records in the index, under the two identities as written.
    let stored: Vec<String> = Store::open(&db)
        .unwrap()
        .records(&id)
        .unwrap()
        .into_iter()
        .map(|record| record.uuid)
        .collect();
    assert!(stored.contains(&bare), "{stored:?}");
    assert!(stored.contains(&padded), "{stored:?}");

    // And two in the payload, with the same bytes.
    let state = state(root.path());
    let carried: Vec<String> = records(open(&state, &id))
        .into_iter()
        .map(|record| record.id)
        .collect();
    assert_eq!(carried.len(), 2);
    assert!(carried.contains(&bare), "{carried:?}");
    assert!(carried.contains(&padded), "{carried:?}");
}

const CODEX: &str = "00000000-0000-4000-8000-00000000c0de";
const CURSOR: &str = "00000000-0000-4000-8000-00000000c0d5";

/// A fake interpreter: it answers the runtime probe, records every run at
/// `marker`, and otherwise runs `body` as the pinned reader.
fn fake_python(root: &Path, marker: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("python3");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\n\
             if [ \"$1\" = \"-c\" ]; then echo True; exit 0; fi\n{body}\n",
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// One whole reader session for `native` of `host`, from a source under the
/// host's root in `home`.
fn reader_session(home: &Path, host: &str, native: &str) -> String {
    let source = match host {
        "codex" => home.join(".codex/sessions/rollout.jsonl"),
        _ => home.join(format!(
            ".cursor/projects/p/agent-transcripts/{native}/{native}.jsonl"
        )),
    };
    [
        json!({
            "type": "session", "host": host, "native_session_id": native,
            "conversation_id": format!("{host}-{native}"), "source_surface": null,
            "started_at": null, "cwd": "/repo/fixture", "git_branch": null, "title": null,
            "path": source.to_str().unwrap(), "mtime": 1788782400.0
        }),
        json!({
            "uuid": uuid(0), "type": "user", "cwd": "/repo/fixture",
            "timestamp": "2026-09-07T12:00:00.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "reader ask"}]}
        }),
        json!({
            "uuid": uuid(1), "type": "assistant", "cwd": "/repo/fixture",
            "timestamp": "2026-09-07T12:00:01.000Z",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "reader reply"}]}
        }),
    ]
    .iter()
    .map(|line| format!("printf '%s\\n' '{line}'\n"))
    .collect()
}

/// An indexed home holding one Codex and one Cursor session's metadata, as
/// the index records them, under the canonical spelling of the temp root.
fn reader_scene() -> (tempfile::TempDir, PathBuf, PathBuf, AppState) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let home = home_with(&root, SESSION, &transcript(SESSION));
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    fs::create_dir_all(home.join(".cursor/projects")).unwrap();
    let db = root.join("data/xtrace.db");
    index(&db, &home);
    {
        let mut store = Store::open(&db).unwrap();
        for (id, host, native) in [
            ("codex-c0de", "codex", CODEX),
            ("cursor-c0d5", "cursor", CURSOR),
        ] {
            let mut meta = SessionMeta::new(id, host, SessionSource::ReadersCli);
            meta.native_session_id = Some(native.to_owned());
            store.upsert_session(&meta, false).unwrap();
        }
    }
    let state = state(&root);
    (temp, root, db, state)
}

fn bundle_with(python: &Path) -> DetailReaders {
    DetailReaders::bundle(
        repo().join("vendor/agent-plugins"),
        Some(python.as_os_str().to_owned()),
    )
}

/// A Codex or Cursor session opens through the pinned reader, asked for by the
/// exact native identity the index stored — never a path, never a search —
/// and crosses as records with no local path in them.
#[test]
fn codex_and_cursor_sessions_open_through_the_pinned_reader_by_their_stored_identity() {
    let (_temp, root, db, state) = reader_scene();
    let home = root.join("home");
    let before = counts(&db);
    for (id, host, native) in [
        ("codex-c0de", "codex", CODEX),
        ("cursor-c0d5", "cursor", CURSOR),
    ] {
        let marker = root.join(format!("{host}.runs"));
        let python = fake_python(&root, &marker, &reader_session(&home, host, native));
        let status = state
            .session_transcript(id, &CancelToken::new(), &bundle_with(&python))
            .unwrap();
        let wire = serde_json::to_string(&status).unwrap();
        assert!(!wire.contains(root.to_str().unwrap()), "{wire}");
        let texts: Vec<String> = records(status)
            .into_iter()
            .flat_map(|record| record.blocks)
            .filter_map(|block| match block {
                SourceBlock::Text { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["reader ask", "reader reply"], "{host}");
        let runs = fs::read_to_string(&marker).unwrap();
        let reader_run = runs.lines().find(|line| !line.starts_with("-c")).unwrap();
        assert!(
            reader_run.ends_with(&format!(
                "--host {host} --exact-detail --session={native} --deadline-seconds 30.000"
            )),
            "{reader_run}"
        );
    }
    assert_eq!(counts(&db), before, "opening a session changed no row");
}

/// Every way the reader can refuse is its own reason, the session's metadata
/// stands, and nothing the reader wrote crosses.
#[test]
fn each_reader_refusal_crosses_as_its_own_reason_with_the_measurements_intact() {
    let (_temp, root, db, state) = reader_scene();
    let home = root.join("home");
    let before = counts(&db);
    let diagnostic = |host: &str, code: &str| {
        format!(
            "printf '%s\\n' '{}' >&2\nexit 2",
            json!({"type": "diagnostic", "host": host, "code": code, "path": "/secret"})
        )
    };
    for (id, body, expected) in [
        (
            "cursor-c0d5",
            diagnostic("cursor", "detail_prerequisite_unsupported"),
            SessionSourceReason::StoreUnsupported,
        ),
        (
            "codex-c0de",
            diagnostic("codex", "detail_limit_native_rows"),
            SessionSourceReason::ReaderLimit {
                limit: ReaderLimit::NativeRows,
            },
        ),
        // Identifying the session's file among the others is its own
        // ceiling, never the selected session's size or a broken answer.
        (
            "codex-c0de",
            diagnostic("codex", "detail_limit_header_probe_bytes"),
            SessionSourceReason::ReaderLimit {
                limit: ReaderLimit::HeaderProbeBytes,
            },
        ),
        (
            "codex-c0de",
            diagnostic("codex", "detail_limit_probe_bytes"),
            SessionSourceReason::ReaderLimit {
                limit: ReaderLimit::ProbeBytes,
            },
        ),
        (
            "codex-c0de",
            diagnostic("codex", "detail_limit_probes"),
            SessionSourceReason::ReaderLimit {
                limit: ReaderLimit::Probes,
            },
        ),
        (
            "codex-c0de",
            diagnostic("codex", "detail_deadline"),
            SessionSourceReason::ReaderDeadline,
        ),
        (
            "codex-c0de",
            format!("{}exit 1", reader_session(&home, "codex", CODEX)),
            SessionSourceReason::ReaderProtocol,
        ),
        (
            // Another session's stream is not this one.
            "codex-c0de",
            reader_session(&home, "codex", "00000000-0000-4000-8000-00000000beef"),
            SessionSourceReason::ReaderProtocol,
        ),
    ] {
        let python = fake_python(&root, &root.join("runs"), &body);
        let status = state
            .session_transcript(id, &CancelToken::new(), &bundle_with(&python))
            .unwrap();
        let wire = serde_json::to_string(&status).unwrap();
        assert!(
            !wire.contains("secret") && !wire.contains("reader ask"),
            "{wire}"
        );
        assert_eq!(reason(status), expected, "{id}: {body}");
    }
    assert_eq!(counts(&db), before);
}

/// When no reader can be given, the reason says which prerequisite is
/// missing, and no reader process is started to find that out.
#[test]
fn a_reader_that_cannot_be_given_says_why_and_starts_nothing() {
    let (_temp, root, _db, state) = reader_scene();
    let marker = root.join("runs");
    let python = fake_python(&root, &marker, "exit 0");
    for (readers, expected) in [
        (DetailReaders::disabled(), ReaderUnavailableCause::Index),
        (
            // A bundle that is not the pinned one.
            DetailReaders::bundle(root.join("home"), Some(python.as_os_str().to_owned())),
            ReaderUnavailableCause::Readers,
        ),
        (
            bundle_with(&root.join("no-such-python")),
            ReaderUnavailableCause::Interpreter,
        ),
    ] {
        assert_eq!(
            reason(
                state
                    .session_transcript("codex-c0de", &CancelToken::new(), &readers)
                    .unwrap()
            ),
            SessionSourceReason::ReaderUnavailable { cause: expected }
        );
    }
    assert!(!marker.exists(), "no interpreter was started");
    // A cancelled open resolves nothing at all.
    let cancel = CancelToken::new();
    cancel.cancel();
    assert_eq!(
        reason(
            state
                .session_transcript("codex-c0de", &cancel, &bundle_with(&python))
                .unwrap()
        ),
        SessionSourceReason::Cancelled
    );
    assert!(!marker.exists(), "a cancelled open probed an interpreter");
}

/// A Claude session never consults the reader, and an identity the read would
/// refuse costs no interpreter probe either.
#[test]
fn only_a_valid_codex_or_cursor_identity_reaches_the_reader() {
    let (_temp, root, db, state) = reader_scene();
    let marker = root.join("runs");
    let python = fake_python(&root, &marker, "exit 0");
    let claude = canonical_id(&db, SESSION);
    records(
        state
            .session_transcript(&claude, &CancelToken::new(), &bundle_with(&python))
            .unwrap(),
    );
    {
        let mut store = Store::open(&db).unwrap();
        let mut meta = SessionMeta::new("codex-latest", "codex", SessionSource::ReadersCli);
        meta.native_session_id = Some("latest".to_owned());
        store.upsert_session(&meta, false).unwrap();
    }
    assert_eq!(
        reason(
            state
                .session_transcript("codex-latest", &CancelToken::new(), &bundle_with(&python))
                .unwrap()
        ),
        SessionSourceReason::InvalidIdentifier
    );
    assert!(!marker.exists(), "{}", fs::read_to_string(&marker).unwrap());
}

/// The environment variable that turns [`app_process_that_quits_mid_read`]
/// into the app half of the shutdown regression; it names the scene's root.
const QUIT_SCENE: &str = "XTRACE_TEST_QUIT_MID_READ";

/// Quitting while a Codex open is still reading leaves no reader behind — in
/// a process that really exits the way the app does.
///
/// The app's exit handler runs `TranscriptReads::shutdown` and returns, and
/// the app then ends its process at once. So this test runs the app half in a
/// separate process: it opens a Codex session whose reader never finishes,
/// calls `shutdown` once the reader is running, and calls
/// `std::process::exit` immediately after — nothing joins the open, nothing
/// keeps the process alive for the supervisor. Only once that process is gone
/// does this one look for the reader, which must be gone too: killed with its
/// group and reaped by its supervisor before `shutdown` returned.
#[test]
fn quitting_mid_read_leaves_no_reader_running_after_the_app_exits() {
    let (_temp, root, _db, _state) = reader_scene();
    let pid_file = root.join("reader.pid");
    fake_python(
        &root,
        &root.join("runs"),
        &format!("echo $$ > '{}'\nexec sleep 1000", pid_file.display()),
    );
    let started = std::time::Instant::now();
    let mut app = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "app_process_that_quits_mid_read",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(QUIT_SCENE, &root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .unwrap();
    let status = loop {
        if let Some(status) = app.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > std::time::Duration::from_secs(30) {
            let _ = app.kill();
            panic!("the app process did not exit");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let mut said = String::new();
    std::io::Read::read_to_string(app.stdout.as_mut().unwrap(), &mut said).unwrap();
    assert_eq!(status.code(), Some(0), "{said}");
    assert!(said.contains("shutdown ended every read"), "{said}");
    // Quitting was bounded: the barrier waited for the cleanup, not longer.
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    let pid: i32 = fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // The reader was the supervisor's child and was reaped before the app
    // exited; an orphan left behind would still be sleeping here.
    // SAFETY: signal 0 only asks whether the process exists.
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    if alive {
        // Clean up after a failure rather than leave a sleeper behind.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
            libc::kill(pid, libc::SIGKILL);
        }
    }
    assert!(!alive, "the reader outlived the app");
}

/// The app half of [`quitting_mid_read_leaves_no_reader_running_after_the_app_exits`].
/// Run on its own it has no scene and does nothing.
#[test]
fn app_process_that_quits_mid_read() {
    use xtrace_desktop::transcript_reads::TranscriptReads;
    let Some(root) = std::env::var_os(QUIT_SCENE).map(PathBuf::from) else {
        return;
    };
    let state = state(&root);
    let readers = bundle_with(&root.join("python3"));
    let reads: &'static TranscriptReads = Box::leak(Box::default());
    let state: &'static AppState = Box::leak(Box::new(state));
    let readers: &'static DetailReaders = Box::leak(Box::new(readers));
    // The open runs on its own thread, as the app's async command does, and
    // is never joined.
    std::thread::spawn(move || {
        let read = reads.begin("open-1").unwrap();
        let _ = state.session_transcript("codex-c0de", read.token(), readers);
    });
    let pid_file = root.join("reader.pid");
    while !pid_file.exists() {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    // What the app's exit handler does, then what the app does after it.
    if reads.shutdown() {
        println!("shutdown ended every read");
        std::process::exit(0);
    }
    println!("shutdown gave up waiting");
    std::process::exit(3);
}
