//! Initial Claude import from a fake `~/.claude/projects` tree: main transcripts
//! and sidechain files map to their canonical session, ignored trees stay
//! ignored, only complete lines import, the default policy keeps metadata only,
//! a repeated import adds nothing, and the source bytes never change.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use xt_ingest::native::{HostStatus, ImportReport, ImportRequest, SessionOutcome, import_native};
use xt_store::{Host, SessionSource, Store};

const SID: &str = "00000000-0000-4000-8000-000000000001";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture_records() -> Vec<Value> {
    fs::read_to_string(repo().join("fixtures/F1/input/sessions/session.jsonl"))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn native_line(mut record: Value, session: &str) -> String {
    let object = record.as_object_mut().unwrap();
    object.insert("sessionId".into(), json!(session));
    object.insert("cwd".into(), json!("/repo/fixture"));
    object.insert("gitBranch".into(), json!("fixture-branch"));
    object.insert("entrypoint".into(), json!("cli"));
    object.insert("version".into(), json!("2.0.0"));
    object.insert("userType".into(), json!("external"));
    record.to_string()
}

/// A complete fake home: one main transcript with 25 fixture records plus one
/// structural line and a partial tail, one sidechain file, and ignored trees.
fn fake_home(home: &Path) -> PathBuf {
    let project = home.join(".claude/projects/-Users-fixture-repo");
    fs::create_dir_all(&project).unwrap();
    let mut main = fixture_records()
        .into_iter()
        .map(|record| native_line(record, SID))
        .collect::<Vec<_>>()
        .join("\n");
    main.push('\n');
    main.push_str(
        &json!({"type":"attachment","sessionId":SID,"attachment":{"type":"synthetic"}}).to_string(),
    );
    main.push('\n');
    main.push_str(r#"{"uuid":"11111111-1111-4111-8111-999999999999","type":"assistant","message":{"role":"assis"#);
    fs::write(project.join(format!("{SID}.jsonl")), main).unwrap();
    let agent = project.join(SID).join("subagents/agent-a");
    fs::create_dir_all(&agent).unwrap();
    let sidechain = [
        json!({"uuid":"22222222-2222-4222-8222-000000000001","type":"user","isSidechain":true,"agentId":"agent-a","parentUuid":"11111111-1111-4111-8111-000000000001","sessionId":SID,"timestamp":"2026-09-07T12:30:00Z","cwd":"/repo/fixture","message":{"role":"user","content":[{"type":"text","text":"Sidechain request."}]}}),
        json!({"uuid":"22222222-2222-4222-8222-000000000002","type":"assistant","isSidechain":true,"agentId":"agent-a","parentUuid":"22222222-2222-4222-8222-000000000001","sessionId":SID,"timestamp":"2026-09-07T12:30:05Z","cwd":"/repo/fixture","message":{"role":"assistant","model":"fixture-model-v1","content":[{"type":"text","text":"Sidechain answer."}],"usage":{"input_tokens":3,"output_tokens":2}}}),
    ];
    fs::write(
        agent.join("agent-a-00000000.jsonl"),
        sidechain
            .iter()
            .map(|r| r.to_string() + "\n")
            .collect::<String>(),
    )
    .unwrap();
    fs::create_dir_all(project.join("memory")).unwrap();
    fs::write(
        project.join("memory/notes.jsonl"),
        "{\"type\":\"user\",\"uuid\":\"memory\"}\n",
    )
    .unwrap();
    fs::write(project.join("README.md"), "not a transcript\n").unwrap();
    fs::write(project.join(".hidden.jsonl"), "{}\n").unwrap();
    project
}

fn hashes(root: &Path) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.insert(
                    path.clone(),
                    format!("{:x}", Sha256::digest(fs::read(&path).unwrap())),
                );
            }
        }
    }
    walk(root, &mut out);
    out
}

fn run(store: &mut Store, home: &Path) -> ImportReport {
    // Model distinct scan starts deterministically; equal-time ordering has
    // dedicated storage coverage.
    static CLOCK: std::sync::atomic::AtomicI64 =
        std::sync::atomic::AtomicI64::new(1_788_782_400_000);
    import_native(
        store,
        &ImportRequest {
            home,
            hosts: &[Host::Claude],
            pin: &repo().join(".plugin-pin"),
            plugin_root: None,
            python: None,
            observed_at: CLOCK.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        },
    )
}

#[test]
fn claude_fs_imports_main_and_sidechain_files_metadata_only_and_leaves_sources_unchanged() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = fake_home(&home);
    let before = hashes(&home);
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    let claude = &report.hosts[0];
    assert_eq!(claude.status, HostStatus::Complete, "{claude:?}");
    assert!(report.complete());
    let outcomes = claude
        .sessions
        .iter()
        .map(|s| (s.path.clone().unwrap(), s.outcome.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes.len(),
        2,
        "main and sidechain files only: {outcomes:?}"
    );
    let main = claude
        .sessions
        .iter()
        .find(|s| {
            s.path
                .as_deref()
                .unwrap()
                .ends_with(&format!("{SID}.jsonl"))
        })
        .unwrap();
    assert_eq!(
        main.outcome,
        SessionOutcome::Imported {
            records_new: 25,
            records_enriched: 0
        }
    );
    assert_eq!(main.source_surface.as_deref(), Some("cli"));
    let side = claude
        .sessions
        .iter()
        .find(|s| s.path.as_deref().unwrap().contains("subagents"))
        .unwrap();
    assert_eq!(
        side.conversation_id.as_deref(),
        Some(SID),
        "sidechains map to the parent session"
    );
    assert_eq!(
        side.outcome,
        SessionOutcome::Imported {
            records_new: 2,
            records_enriched: 0
        }
    );

    let session = store.session(SID).unwrap().expect("session row");
    assert_eq!(session.meta.host, Host::Claude);
    assert_eq!(session.meta.source, SessionSource::Transcript);
    assert_eq!(session.meta.native_session_id.as_deref(), Some(SID));
    assert_eq!(session.meta.surface.as_deref(), Some("cli"));
    assert_eq!(session.meta.cwd.as_deref(), Some("/repo/fixture"));
    assert_eq!(session.meta.git_branch.as_deref(), Some("fixture-branch"));
    assert_eq!(
        session.meta.title, None,
        "titles are never acquired from native history"
    );
    assert_eq!(
        session.meta.started_at_ms, None,
        "Claude files carry no native start; unknown stays unknown"
    );
    let records = store.records(SID).unwrap();
    assert_eq!(
        records.len(),
        27,
        "25 main records plus two sidechain records; the partial tail is not imported"
    );
    assert!(
        records.iter().all(|r| r.content_json.is_none()),
        "metadata-only default stores no content"
    );
    assert!(
        records
            .iter()
            .all(|r| r.tool_uses.iter().all(|t| t.input_json.is_none()))
    );
    assert!(
        records.iter().any(|r| r.tool_use_count.unwrap_or(0) > 0),
        "tool counts are kept"
    );
    assert!(
        records.iter().any(|r| r.usage.is_some()),
        "readable usage is kept"
    );
    assert!(
        records.iter().filter(|r| r.usage.is_none()).count() > 0,
        "absent usage stays unknown"
    );
    let sidechains = records
        .iter()
        .filter(|r| r.is_sidechain)
        .collect::<Vec<_>>();
    assert_eq!(sidechains.len(), 2);
    assert!(
        sidechains
            .iter()
            .all(|r| r.identity.agent_id.as_deref() == Some("agent-a"))
    );
    assert!(
        records
            .iter()
            .any(|r| r.model.as_deref() == Some("fixture-model-v1"))
    );
    assert_eq!(store.counts().unwrap().sessions, 1);

    let main_path = project.join(format!("{SID}.jsonl"));
    let cursor = store
        .source_cursor(
            SessionSource::Transcript,
            &format!("claude:{}", main_path.display()),
        )
        .unwrap()
        .expect("main file cursor");
    let bytes = fs::read(&main_path).unwrap();
    let complete = bytes.iter().rposition(|b| *b == b'\n').unwrap() as i64 + 1;
    assert_eq!(
        cursor.position, complete,
        "the cursor stops before the partial tail"
    );
    assert!(cursor.position < bytes.len() as i64);
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].native_session_id, SID);
    assert_eq!(discovered[0].surface.as_deref(), Some("cli"));
    assert!(discovered[0].discovery_complete);

    let again = run(&mut store, &home);
    assert!(again.complete());
    for session in &again.hosts[0].sessions {
        assert_eq!(
            session.outcome,
            SessionOutcome::Imported {
                records_new: 0,
                records_enriched: 0
            },
            "a repeated import adds nothing: {session:?}"
        );
    }
    assert_eq!(store.records(SID).unwrap().len(), 27);
    assert_eq!(
        hashes(&home),
        before,
        "source bytes and the set of files are unchanged"
    );
}

#[test]
fn claude_fs_reports_absent_roots_and_identity_disagreements_explicitly() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::MissingSource);
    assert!(!report.complete());

    let project = fake_home(&home);
    // A file whose records belong to another session cannot be imported under its name.
    let other = "00000000-0000-4000-8000-000000000002";
    fs::write(
        project.join(format!("{other}.jsonl")),
        fixture_records()
            .into_iter()
            .take(3)
            .map(|r| native_line(r, SID) + "\n")
            .collect::<String>(),
    )
    .unwrap();
    let report = run(&mut store, &home);
    let claude = &report.hosts[0];
    assert_eq!(claude.status, HostStatus::Incomplete);
    let skipped = claude
        .sessions
        .iter()
        .find(|s| s.native_session_id.as_deref() == Some(other))
        .unwrap();
    match &skipped.outcome {
        SessionOutcome::Skipped { reason } => assert!(reason.contains("identity"), "{reason}"),
        other => panic!("expected an explicit skip, got {other:?}"),
    }
    assert!(
        store.session(other).unwrap().is_none(),
        "nothing is written for the skipped file"
    );
    assert_eq!(
        store.records(SID).unwrap().len(),
        27,
        "readable sessions still import"
    );

    // Bytes that are not UTF-8 are never decoded lossily into a session.
    let binary = "00000000-0000-4000-8000-000000000003";
    fs::write(
        project.join(format!("{binary}.jsonl")),
        b"{\"type\":\"user\",\"uuid\":\"\xff\xfe\"}\n",
    )
    .unwrap();
    let report = run(&mut store, &home);
    let skipped = report.hosts[0]
        .sessions
        .iter()
        .find(|s| s.native_session_id.as_deref() == Some(binary))
        .unwrap();
    match &skipped.outcome {
        SessionOutcome::Skipped { reason } => assert!(reason.contains("UTF-8"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert!(store.session(binary).unwrap().is_none());
    // Both skipped files are still known sessions: their file names identify
    // them, and that identity is registered before any line is parsed.
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    for id in [other, binary] {
        let known = discovered
            .iter()
            .find(|d| d.native_session_id == id)
            .unwrap_or_else(|| panic!("{id} is discovered"));
        assert_eq!(known.surface, None, "nothing was read from {id}");
        assert!(known.discovery_complete);
    }
}

#[test]
fn claude_fs_registers_identities_before_parsing_and_enriches_them_from_the_first_batch() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-early");
    fs::create_dir_all(&project).unwrap();
    let broken = "00000000-0000-4000-8000-00000000b001";
    let binary = "00000000-0000-4000-8000-00000000b002";
    let sound = "00000000-0000-4000-8000-00000000b003";
    // Malformed JSON on the very first line, before any record.
    fs::write(
        project.join(format!("{broken}.jsonl")),
        "{\"type\":\"user\",\"uuid\":5}\n",
    )
    .unwrap();
    fs::write(
        project.join(format!("{binary}.jsonl")),
        b"{\"type\":\"user\",\"uuid\":\"\xff\"}\n",
    )
    .unwrap();
    fs::write(
        project.join(format!("{sound}.jsonl")),
        fixture_records()
            .into_iter()
            .take(3)
            .map(|r| native_line(r, sound) + "\n")
            .collect::<String>(),
    )
    .unwrap();
    let before = hashes(&home);
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Incomplete, "{report:?}");
    for id in [broken, binary] {
        let skipped = report.hosts[0]
            .sessions
            .iter()
            .find(|s| s.native_session_id.as_deref() == Some(id))
            .unwrap();
        match &skipped.outcome {
            SessionOutcome::Skipped { reason } => assert!(
                reason.contains("stream line 1") && reason.contains("0 earlier batches"),
                "{reason}"
            ),
            other => panic!("{other:?}"),
        }
        assert!(store.session(id).unwrap().is_none());
    }
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    assert_eq!(discovered.len(), 3, "{discovered:?}");
    let find = |id: &str| {
        discovered
            .iter()
            .find(|d| d.native_session_id == id)
            .unwrap()
    };
    assert_eq!(find(broken).surface, None);
    assert_eq!(find(binary).surface, None);
    assert_eq!(
        find(sound).surface.as_deref(),
        Some("cli"),
        "the first batch enriches the registered identity"
    );
    assert_eq!(store.counts().unwrap().sessions, 1);
    assert_eq!(hashes(&home), before);
}

fn synthetic_line(index: usize, session: &str) -> String {
    let role = if index.is_multiple_of(2) {
        "user"
    } else {
        "assistant"
    };
    // A UUID namespace per session: the last four session characters keep two
    // synthetic files from sharing record identities.
    let tail = &session[session.len() - 4..];
    json!({
        "uuid": format!("4444{tail}-4444-4444-8444-{index:012}"),
        "type": role,
        "sessionId": session,
        "entrypoint": "cli",
        "cwd": "/repo/fixture",
        "timestamp": format!("2026-09-07T12:{:02}:{:02}Z", (index / 60) % 60, index % 60),
        "message": {"role": role, "content": [{"type": "text", "text": format!("turn {index}")}]}
    })
    .to_string()
}

#[test]
fn claude_fs_streams_large_files_in_bounded_batches_and_keeps_committed_batches_on_a_later_fault() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-large");
    fs::create_dir_all(&project).unwrap();
    let big = "00000000-0000-4000-8000-00000000b16e";
    let path = project.join(format!("{big}.jsonl"));
    let body = (0..4_500)
        .map(|i| synthetic_line(i, big) + "\n")
        .collect::<String>();
    fs::write(&path, &body).unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
    assert_eq!(
        report.hosts[0].sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 4_500,
            records_enriched: 0
        }
    );
    assert_eq!(store.records(big).unwrap().len(), 4_500);
    let cursor = store
        .source_cursor(
            SessionSource::Transcript,
            &format!("claude:{}", path.display()),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        cursor.position,
        body.len() as i64,
        "the cursor commits with the final batch"
    );
    assert_eq!(
        store.session(big).unwrap().unwrap().meta.surface.as_deref(),
        Some("cli")
    );
    let again = run(&mut store, &home);
    assert_eq!(
        again.hosts[0].sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 0,
            records_enriched: 0
        }
    );

    // A fault after the first committed batch: the earlier batch stays, the
    // outcome says so, and the cursor is not advanced for that file.
    let faulty = "00000000-0000-4000-8000-00000000fa17";
    let path = project.join(format!("{faulty}.jsonl"));
    let mut lines = (0..2_500)
        .map(|i| synthetic_line(i, faulty))
        .collect::<Vec<_>>();
    lines[2_100] = r#"{"type":"assistant","uuid":7}"#.into();
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    let report = run(&mut store, &home);
    let session = report.hosts[0]
        .sessions
        .iter()
        .find(|s| s.native_session_id.as_deref() == Some(faulty))
        .unwrap();
    match &session.outcome {
        SessionOutcome::Skipped { reason } => {
            assert!(
                reason.contains("stream line 2101")
                    && reason.contains("1 earlier batches stay committed"),
                "{reason}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        store.records(faulty).unwrap().len(),
        2_000,
        "the committed batch remains"
    );
    assert!(
        store
            .source_cursor(
                SessionSource::Transcript,
                &format!("claude:{}", path.display())
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(report.hosts[0].status, HostStatus::Incomplete);
}

#[cfg(unix)]
#[test]
fn claude_fs_continues_past_an_unreadable_project_directory() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    fake_home(&home);
    let locked = home.join(".claude/projects/-Users-locked");
    fs::create_dir_all(&locked).unwrap();
    fs::write(
        locked.join("00000000-0000-4000-8000-00000000d00d.jsonl"),
        synthetic_line(0, "00000000-0000-4000-8000-00000000d00d") + "\n",
    )
    .unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let claude = &report.hosts[0];
    assert_eq!(claude.status, HostStatus::Incomplete, "{claude:?}");
    assert!(
        claude
            .diagnostics
            .iter()
            .any(|d| d.code == "discovery_incomplete"
                && d.path.as_deref().unwrap().ends_with("-Users-locked")),
        "{:?}",
        claude.diagnostics
    );
    assert_eq!(
        store.records(SID).unwrap().len(),
        27,
        "the readable project still imports in full"
    );
}

#[cfg(unix)]
#[test]
fn claude_fs_keeps_an_exact_batch_boundary_cursor_and_reports_aliases() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-exact");
    fs::create_dir_all(&project).unwrap();
    let exact = "00000000-0000-4000-8000-00000000e2ac";
    let path = project.join(format!("{exact}.jsonl"));
    let body = (0..2_000)
        .map(|i| synthetic_line(i, exact) + "\n")
        .collect::<String>();
    fs::write(&path, &body).unwrap();
    // An alias beside it is reported, never followed.
    std::os::unix::fs::symlink(
        &path,
        project.join("00000000-0000-4000-8000-00000000a11a.jsonl"),
    )
    .unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    let claude = &report.hosts[0];
    assert_eq!(claude.status, HostStatus::Incomplete, "{claude:?}");
    assert!(
        claude
            .diagnostics
            .iter()
            .any(|d| d.path.as_deref().unwrap().ends_with("a11a.jsonl"))
    );
    assert_eq!(claude.sessions.len(), 1);
    assert_eq!(
        claude.sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 2_000,
            records_enriched: 0
        }
    );
    let cursor = store
        .source_cursor(
            SessionSource::Transcript,
            &format!("claude:{}", path.display()),
        )
        .unwrap()
        .expect("a file ending exactly on a batch boundary keeps its cursor");
    assert_eq!(cursor.position, body.len() as i64);
    assert!(
        store
            .session("00000000-0000-4000-8000-00000000a11a")
            .unwrap()
            .is_none()
    );
}

#[test]
fn claude_fs_registers_empty_and_inert_only_files_as_discovered_without_rows() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-empty");
    fs::create_dir_all(&project).unwrap();
    let empty = "00000000-0000-4000-8000-00000000e000";
    let inert = "00000000-0000-4000-8000-00000000e001";
    fs::write(project.join(format!("{empty}.jsonl")), "").unwrap();
    fs::write(
        project.join(format!("{inert}.jsonl")),
        format!(
            "{}\n",
            json!({"type": "summary", "summary": "x", "leafUuid": "u", "sessionId": inert})
        ),
    )
    .unwrap();
    let before = hashes(&home);
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
    assert_eq!(report.hosts[0].sessions.len(), 2);
    for session in &report.hosts[0].sessions {
        assert_eq!(
            session.outcome,
            SessionOutcome::Imported {
                records_new: 0,
                records_enriched: 0
            },
            "{session:?}"
        );
    }
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    let mut ids: Vec<&str> = discovered
        .iter()
        .map(|d| d.native_session_id.as_str())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![empty, inert]);
    assert_eq!(
        store.counts().unwrap().sessions,
        0,
        "a file without a storable record leaves no canonical row"
    );
    for id in [empty, inert] {
        let key = format!("claude:{}", project.join(format!("{id}.jsonl")).display());
        assert!(
            store
                .source_cursor(SessionSource::Transcript, &key)
                .unwrap()
                .is_none(),
            "{key} must not record a cursor"
        );
    }
    assert_eq!(hashes(&home), before);
}

/// A spelling of `target` relative to the current directory, via `..` hops.
fn relative_spelling(target: &Path) -> String {
    let cwd = std::env::current_dir().unwrap();
    let ups = "../".repeat(cwd.components().count() - 1);
    let relative = format!(
        "{ups}{}",
        target
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .trim_start_matches('/')
    );
    assert!(Path::new(&relative).is_relative());
    relative
}

#[test]
fn claude_fs_anchors_a_relative_home_so_paths_and_cursor_keys_are_absolute() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = fake_home(&home);
    let relative = relative_spelling(&home);
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, Path::new(&relative));
    assert!(report.complete(), "{report:?}");
    let anchored = std::env::current_dir().unwrap().join(&relative);
    let main = report.hosts[0]
        .sessions
        .iter()
        .find(|s| {
            s.path
                .as_deref()
                .is_some_and(|p| p.ends_with(&format!("{SID}.jsonl")))
        })
        .unwrap();
    let path = Path::new(main.path.as_deref().unwrap());
    assert!(path.is_absolute(), "{path:?}");
    assert_eq!(
        path.canonicalize().unwrap(),
        project.join(format!("{SID}.jsonl")).canonicalize().unwrap()
    );
    assert!(path.starts_with(&anchored), "{path:?} under {anchored:?}");
    let key = format!("claude:{}", path.display());
    assert!(
        store
            .source_cursor(SessionSource::Transcript, &key)
            .unwrap()
            .is_some(),
        "the cursor key uses the anchored path"
    );
    // The absolute spelling of the same home is the same import: nothing new.
    let again = run(&mut store, &home);
    assert!(again.complete());
    assert!(again.hosts[0].sessions.iter().all(|s| matches!(
        s.outcome,
        SessionOutcome::Imported {
            records_new: 0,
            records_enriched: 0
        }
    )));
    // The sidechain belongs to the parent session: one canonical session.
    assert_eq!(store.counts().unwrap().sessions, 1);
}

#[test]
fn claude_fs_never_persists_a_blank_record_label_as_the_discovered_surface() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-blank");
    fs::create_dir_all(&project).unwrap();
    let session = "00000000-0000-4000-8000-00000000b1a0";
    let file = project.join(format!("{session}.jsonl"));
    let records = fixture_records().into_iter().take(2).collect::<Vec<_>>();
    let write = |entrypoint: &str| {
        let body = records
            .iter()
            .cloned()
            .map(|record| {
                // `native_line` fixes the surface and cwd; override them after.
                let mut line: Value = serde_json::from_str(&native_line(record, session)).unwrap();
                line["entrypoint"] = json!(entrypoint);
                line.to_string() + "\n"
            })
            .collect::<String>();
        fs::write(&file, body).unwrap();
    };
    // The writer rejects a blank identity label: the file is skipped
    // explicitly, and the discovered identity stays without a surface.
    write(" ");
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Incomplete, "{report:?}");
    match &report.hosts[0].sessions[0].outcome {
        SessionOutcome::Skipped { reason } => assert!(reason.contains("empty"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert!(store.session(session).unwrap().is_none());
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].surface, None, "a blank label is no label");

    // A corrected transcript imports afterwards and fills the surface in.
    write("cli");
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
    assert_eq!(
        report.hosts[0].sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 2,
            records_enriched: 0
        }
    );
    assert!(store.session(session).unwrap().is_some());
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    assert_eq!(discovered[0].surface.as_deref(), Some("cli"));
}

#[test]
fn claude_fs_keeps_enriching_metadata_after_the_first_batch() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-late");
    fs::create_dir_all(&project).unwrap();
    let session = "00000000-0000-4000-8000-00000000d0d0";
    let mut body = String::new();
    for index in 0..2_005 {
        let mut line: Value = serde_json::from_str(&synthetic_line(index, session)).unwrap();
        if index < 2_000 {
            // The whole first batch omits every optional label.
            line.as_object_mut().unwrap().remove("entrypoint");
            line.as_object_mut().unwrap().remove("cwd");
        } else {
            line["gitBranch"] = json!("late-branch");
        }
        body.push_str(&line.to_string());
        body.push('\n');
    }
    fs::write(project.join(format!("{session}.jsonl")), body).unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
    let result = &report.hosts[0].sessions[0];
    assert_eq!(
        result.outcome,
        SessionOutcome::Imported {
            records_new: 2_005,
            records_enriched: 0
        }
    );
    assert_eq!(
        result.source_surface.as_deref(),
        Some("cli"),
        "the later batch revealed the surface"
    );
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].surface.as_deref(), Some("cli"));
    let stored = store.session(session).unwrap().unwrap();
    assert_eq!(stored.meta.surface.as_deref(), Some("cli"));
    assert_eq!(stored.meta.cwd.as_deref(), Some("/repo/fixture"));
    assert_eq!(stored.meta.git_branch.as_deref(), Some("late-branch"));
}

#[test]
fn claude_fs_stops_at_a_disagreeing_surface_before_discovery_learns_either() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-surfaces");
    fs::create_dir_all(&project).unwrap();
    let session = "00000000-0000-4000-8000-00000000c0c0";
    let file = project.join(format!("{session}.jsonl"));
    let write = |surfaces: &[&str]| {
        let body = surfaces
            .iter()
            .enumerate()
            .map(|(index, surface)| {
                let mut line: Value =
                    serde_json::from_str(&synthetic_line(index, session)).unwrap();
                line["entrypoint"] = json!(surface);
                line.to_string() + "\n"
            })
            .collect::<String>();
        fs::write(&file, body).unwrap();
    };
    // Two valid but different surfaces in one file: an explicit stop at the
    // second record, and the discovered identity learns neither.
    write(&["cli", "sdk", "sdk"]);
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Incomplete, "{report:?}");
    match &report.hosts[0].sessions[0].outcome {
        SessionOutcome::Skipped { reason } => assert!(
            reason.contains("surface") && reason.contains("stream line 2"),
            "{reason}"
        ),
        other => panic!("{other:?}"),
    }
    assert!(store.session(session).unwrap().is_none());
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].surface, None);

    // The corrected transcript, on the surface the first import did not
    // pick, imports and fills the surface in.
    write(&["sdk", "sdk", "sdk"]);
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
    assert_eq!(
        report.hosts[0].sessions[0].source_surface.as_deref(),
        Some("sdk")
    );
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    assert_eq!(discovered[0].surface.as_deref(), Some("sdk"));
    assert_eq!(
        store
            .session(session)
            .unwrap()
            .unwrap()
            .meta
            .surface
            .as_deref(),
        Some("sdk")
    );
}

#[test]
fn claude_fs_validates_a_batch_before_enriching_discovery() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-platform");
    fs::create_dir_all(&project).unwrap();
    let session = "00000000-0000-4000-8000-00000000c1c1";
    let file = project.join(format!("{session}.jsonl"));
    let write = |surface: &str, platform: Option<&str>| {
        let body = (0..3)
            .map(|index| {
                let mut line: Value =
                    serde_json::from_str(&synthetic_line(index, session)).unwrap();
                line["entrypoint"] = json!(surface);
                if let Some(platform) = platform {
                    line["source_platform"] = json!(platform);
                }
                line.to_string() + "\n"
            })
            .collect::<String>();
        fs::write(&file, body).unwrap();
    };
    // A valid surface next to an identity field the writer rejects: the
    // batch is validated first, so the surface never reaches discovery.
    write("cli", Some("codex"));
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Incomplete, "{report:?}");
    match &report.hosts[0].sessions[0].outcome {
        SessionOutcome::Skipped { reason } => {
            assert!(reason.contains("disagree"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    assert!(store.session(session).unwrap().is_none());
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].surface, None);

    // The corrected transcript, on another valid surface, imports.
    write("sdk", None);
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
    assert_eq!(
        store.discovered_sessions(Host::Claude).unwrap()[0]
            .surface
            .as_deref(),
        Some("sdk")
    );
}

#[test]
fn claude_fs_registers_an_unopenable_transcript_as_discovered() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-locked");
    fs::create_dir_all(&project).unwrap();
    let session = "00000000-0000-4000-8000-00000000c2c2";
    let file = project.join(format!("{session}.jsonl"));
    fs::write(&file, synthetic_line(0, session) + "\n").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    let claude = &report.hosts[0];
    assert_eq!(claude.status, HostStatus::Incomplete, "{report:?}");
    assert!(
        claude
            .diagnostics
            .iter()
            .any(|d| d.code == "session_unreadable"),
        "{report:?}"
    );
    match &claude.sessions[0].outcome {
        SessionOutcome::Skipped { reason } => {
            assert!(reason.contains("could not be read"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    assert_eq!(
        discovered.len(),
        1,
        "the file name still identifies the session"
    );
    assert_eq!(discovered[0].native_session_id, session);
    assert_eq!(store.counts().unwrap().sessions, 0);
}

#[test]
fn claude_fs_persists_labels_only_with_a_committed_batch() {
    use xt_store::ingest::DiscoveredSession;
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-atomic");
    fs::create_dir_all(&project).unwrap();
    let session = "00000000-0000-4000-8000-00000000c3c3";
    let file = project.join(format!("{session}.jsonl"));
    let write = |surface: &str| {
        let body = (0..3)
            .map(|index| {
                let mut line: Value =
                    serde_json::from_str(&synthetic_line(index, session)).unwrap();
                line["entrypoint"] = json!(surface);
                line.to_string() + "\n"
            })
            .collect::<String>();
        fs::write(&file, body).unwrap();
    };
    // The index already knows this session under another surface (as a
    // reader header would have registered it). The parser and the writer's
    // identity checks accept the file; the store rejects the batch when the
    // label it carries conflicts, inside the batch's own transaction.
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    store
        .observe_discovered_session(&DiscoveredSession {
            host: Host::Claude,
            native_session_id: session.to_owned(),
            conversation_id: Some(session.to_owned()),
            surface: Some("sdk".to_owned()),
            started_at_ms: None,
            last_observed_at: 1,
            discovery_complete: true,
        })
        .unwrap();
    write("cli");
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Incomplete, "{report:?}");
    match &report.hosts[0].sessions[0].outcome {
        SessionOutcome::Skipped { reason } => assert!(
            reason.contains("could not be written after 0 committed batches")
                && reason.contains("conflicting discovered session identity"),
            "{reason}"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        report.hosts[0].sessions[0].source_surface, None,
        "a label is reported only once it is persisted"
    );
    assert!(
        store.session(session).unwrap().is_none(),
        "the rejected batch wrote no row"
    );
    assert_eq!(store.records(session).unwrap().len(), 0);
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].surface.as_deref(), Some("sdk"));

    // The transcript on the surface the index knows imports, and the labels
    // fill in with the committed batch.
    write("sdk");
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
    assert_eq!(
        report.hosts[0].sessions[0].source_surface.as_deref(),
        Some("sdk")
    );
    assert_eq!(store.records(session).unwrap().len(), 3);
    let stored = store.session(session).unwrap().unwrap();
    assert_eq!(stored.meta.cwd.as_deref(), Some("/repo/fixture"));
}

#[test]
fn claude_fs_settles_the_surface_over_the_whole_file_before_writing_any_row() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-late-disagreement");
    fs::create_dir_all(&project).unwrap();
    let session = "00000000-0000-4000-8000-00000000c4c4";
    let file = project.join(format!("{session}.jsonl"));
    let write = |surface_at: &dyn Fn(usize) -> &'static str| {
        let body = (0..2_001)
            .map(|index| {
                let mut line: Value =
                    serde_json::from_str(&synthetic_line(index, session)).unwrap();
                line["entrypoint"] = json!(surface_at(index));
                line.to_string() + "\n"
            })
            .collect::<String>();
        fs::write(&file, body).unwrap();
    };
    // A full first batch on one surface, then a record on another: nothing
    // is written, so neither the canonical row nor the discovered identity
    // learns the first surface.
    write(&|index| if index < 2_000 { "cli" } else { "sdk" });
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Incomplete, "{report:?}");
    match &report.hosts[0].sessions[0].outcome {
        SessionOutcome::Skipped { reason } => assert!(
            reason.contains("surface")
                && reason.contains("stream line 2001")
                && reason.contains("0 earlier batches"),
            "{reason}"
        ),
        other => panic!("{other:?}"),
    }
    assert!(
        store.session(session).unwrap().is_none(),
        "no row was written"
    );
    assert_eq!(store.records(session).unwrap().len(), 0);
    let discovered = store.discovered_sessions(Host::Claude).unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].surface, None);

    // The corrected transcript, entirely on the other surface, imports.
    write(&|_| "sdk");
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
    assert_eq!(
        report.hosts[0].sessions[0].outcome,
        SessionOutcome::Imported {
            records_new: 2_001,
            records_enriched: 0
        }
    );
    assert_eq!(
        store.discovered_sessions(Host::Claude).unwrap()[0]
            .surface
            .as_deref(),
        Some("sdk")
    );
    assert_eq!(
        store
            .session(session)
            .unwrap()
            .unwrap()
            .meta
            .surface
            .as_deref(),
        Some("sdk")
    );
}

#[test]
fn claude_fs_never_fills_session_metadata_with_blank_record_labels() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/-Users-blank-meta");
    fs::create_dir_all(&project).unwrap();
    let session = "00000000-0000-4000-8000-00000000c5c5";
    let file = project.join(format!("{session}.jsonl"));
    let write = |cwd: &str, branch: &str| {
        let body = (0..3)
            .map(|index| {
                let mut line: Value =
                    serde_json::from_str(&synthetic_line(index, session)).unwrap();
                line["cwd"] = json!(cwd);
                line["gitBranch"] = json!(branch);
                line.to_string() + "\n"
            })
            .collect::<String>();
        fs::write(&file, body).unwrap();
    };
    // Whitespace-only labels import as absent, never as blank fill-once values.
    write(" ", "");
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
    let stored = store.session(session).unwrap().unwrap();
    assert_eq!(stored.meta.cwd, None, "a blank cwd is no label");
    assert_eq!(stored.meta.git_branch, None, "a blank branch is no label");

    // Real values arriving later fill the metadata in.
    write("/repo/real", "main");
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
    let stored = store.session(session).unwrap().unwrap();
    assert_eq!(stored.meta.cwd.as_deref(), Some("/repo/real"));
    assert_eq!(stored.meta.git_branch.as_deref(), Some("main"));
}

#[test]
fn claude_fs_rescans_a_shorter_replacement_without_sticking_or_losing_history() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/replacement");
    fs::create_dir_all(&project).unwrap();
    let file = project.join(format!("{SID}.jsonl"));
    let key = format!("claude:{}", file.display());
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let body = |count| {
        (0..count)
            .map(|i| synthetic_line(i, SID) + "\n")
            .collect::<String>()
    };
    fs::write(&file, body(5)).unwrap();
    assert!(run(&mut store, &home).complete());
    let old = store
        .source_cursor(SessionSource::Transcript, &key)
        .unwrap()
        .unwrap();
    // Atomic replacement at the same path, shorter than the old cursor.
    let replacement = project.join("replacement.tmp");
    fs::write(&replacement, body(2)).unwrap();
    fs::rename(&replacement, &file).unwrap();
    let before = hashes(&home);
    for _ in 0..2 {
        assert!(run(&mut store, &home).complete());
        let current = store
            .source_cursor(SessionSource::Transcript, &key)
            .unwrap()
            .unwrap();
        assert_eq!(current.position, body(2).len() as i64);
        assert!(current.position < old.position);
        assert_eq!(store.records(SID).unwrap().len(), 5);
    }
    assert_eq!(hashes(&home), before);
    // In-place truncation is also a complete scan; a malformed replacement is not.
    fs::write(&file, body(1)).unwrap();
    assert!(run(&mut store, &home).complete());
    let completed = store
        .source_cursor(SessionSource::Transcript, &key)
        .unwrap()
        .unwrap();
    fs::write(&file, "{broken\n").unwrap();
    assert!(!run(&mut store, &home).complete());
    assert_eq!(
        store
            .source_cursor(SessionSource::Transcript, &key)
            .unwrap()
            .unwrap(),
        completed
    );
    fs::write(&file, body(6)).unwrap();
    assert!(run(&mut store, &home).complete());
    assert_eq!(store.records(SID).unwrap().len(), 6);
    for (index, empty) in ["", "{\"type\":\"attachment\"}\n"].into_iter().enumerate() {
        fs::write(&file, empty).unwrap();
        let before = hashes(&home);
        for _ in 0..2 {
            assert!(run(&mut store, &home).complete());
            assert!(
                store
                    .source_cursor(SessionSource::Transcript, &key)
                    .unwrap()
                    .is_some_and(|cursor| cursor.position == 0)
            );
            assert_eq!(store.records(SID).unwrap().len(), 6 + index);
        }
        assert_eq!(hashes(&home), before);
        // Newly appended content is imported even while shorter than the old file.
        fs::write(&file, synthetic_line(6, SID) + "\n").unwrap();
        assert!(run(&mut store, &home).complete());
        assert_eq!(store.records(SID).unwrap().len(), 7);
        assert!(
            store
                .source_cursor(SessionSource::Transcript, &key)
                .unwrap()
                .is_some()
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn claude_fs_reports_non_utf8_main_and_sidechain_names_without_mutating_sources() {
    use std::os::unix::ffi::OsStringExt;
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = fake_home(&home);
    let sidechain = project.join(SID).join("subagents");
    for directory in [&project, &sidechain] {
        let name = std::ffi::OsString::from_vec(b"invalid-\xff.jsonl".to_vec());
        fs::write(directory.join(name), synthetic_line(99, SID) + "\n").unwrap();
    }
    let before = hashes(&home);
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Incomplete);
    assert_eq!(
        report.hosts[0]
            .diagnostics
            .iter()
            .filter(|d| d.code == "discovery_incomplete")
            .count(),
        2
    );
    assert!(
        !store.records(SID).unwrap().is_empty(),
        "readable files still import"
    );
    assert_eq!(hashes(&home), before);
}

#[cfg(unix)]
#[test]
fn claude_fs_reports_project_aliases_without_following_them() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    fake_home(&home);
    let external = temp.path().join("external");
    fs::create_dir_all(&external).unwrap();
    let transcript = external.join(format!("{SID}.jsonl"));
    let bytes = synthetic_line(99, SID) + "\n";
    fs::write(&transcript, &bytes).unwrap();
    let alias = home.join(".claude/projects/aliased-project");
    symlink(&external, &alias).unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let report = run(&mut store, &home);
    assert_eq!(report.hosts[0].status, HostStatus::Incomplete);
    assert!(
        report.hosts[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "discovery_incomplete" && d.path.as_deref() == alias.to_str())
    );
    assert_eq!(store.records(SID).unwrap().len(), 27);
    assert_eq!(fs::read_to_string(&transcript).unwrap(), bytes);
}

#[cfg(unix)]
#[test]
fn claude_fs_reports_aliased_source_roots_without_reading_external_history() {
    use std::os::unix::fs::symlink;
    for parent in [false, true] {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("home");
        let external = temp.path().join("external");
        fake_home(&external);
        fs::create_dir_all(&home).unwrap();
        let alias = if parent {
            home.join(".claude")
        } else {
            fs::create_dir_all(home.join(".claude")).unwrap();
            home.join(".claude/projects")
        };
        let target = if parent {
            external.join(".claude")
        } else {
            external.join(".claude/projects")
        };
        symlink(target, &alias).unwrap();
        let before = hashes(&external);
        let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
        let report = run(&mut store, &home);
        assert_eq!(report.hosts[0].status, HostStatus::Incomplete);
        assert_eq!(report.hosts[0].diagnostics.len(), 1);
        assert_eq!(
            report.hosts[0].diagnostics[0].path.as_deref(),
            alias.to_str()
        );
        assert_eq!(store.counts().unwrap().records, 0);
        assert_eq!(hashes(&external), before);
    }
}

#[test]
fn claude_fs_rejects_blank_filename_and_parent_identities_before_discovery() {
    for body in ["", "{\"type\":\"attachment\"}\n"] {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("home");
        let project = home.join(".claude/projects/project");
        fs::create_dir_all(project.join(" /subagents")).unwrap();
        fs::write(project.join(" .jsonl"), body).unwrap();
        fs::write(project.join(" /subagents/agent.jsonl"), body).unwrap();
        let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
        let report = run(&mut store, &home);
        assert_eq!(report.hosts[0].status, HostStatus::Incomplete);
        assert_eq!(report.hosts[0].diagnostics.len(), 2);
        assert!(store.discovered_sessions(Host::Claude).unwrap().is_empty());
        assert_eq!(store.counts().unwrap().sessions, 0);
    }
}

#[cfg(unix)]
#[test]
fn claude_fs_distinguishes_unreadable_roots_from_absence() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let root = home.join(".claude");
    fs::create_dir_all(root.join("projects")).unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();
    let report = run(&mut store, &home);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(report.hosts[0].status, HostStatus::ReaderFailed);
    assert!(
        report.hosts[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("permission denied")
    );
    assert_eq!(store.counts().unwrap().records, 0);
    fs::remove_dir(root.join("projects")).unwrap();
    assert_eq!(
        run(&mut store, &home).hosts[0].status,
        HostStatus::MissingSource
    );
}

#[test]
fn claude_fs_checks_surface_on_uuid_less_records_before_persisting_labels() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = home.join(".claude/projects/project");
    fs::create_dir_all(&project).unwrap();
    let file = project.join(format!("{SID}.jsonl"));
    let mut first: Value = serde_json::from_str(&synthetic_line(0, SID)).unwrap();
    first["entrypoint"] = json!("cli");
    let mut second: Value = serde_json::from_str(&synthetic_line(1, SID)).unwrap();
    second["entrypoint"] = json!("sdk");
    second.as_object_mut().unwrap().remove("uuid");
    fs::write(&file, format!("{first}\n{second}\n")).unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    assert!(!run(&mut store, &home).complete());
    assert_eq!(store.counts().unwrap().records, 0);
    assert_eq!(
        store.discovered_sessions(Host::Claude).unwrap()[0].surface,
        None
    );
    first["entrypoint"] = json!("sdk");
    fs::write(&file, format!("{first}\n")).unwrap();
    assert!(run(&mut store, &home).complete());
    assert_eq!(
        store.discovered_sessions(Host::Claude).unwrap()[0]
            .surface
            .as_deref(),
        Some("sdk")
    );
}

#[test]
fn claude_fs_unusable_dropped_surfaces_prevent_cross_batch_label_persistence() {
    for surface in [json!(" "), json!(12), json!({})] {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("home");
        let project = home.join(".claude/projects/project");
        fs::create_dir_all(&project).unwrap();
        let file = project.join(format!("{SID}.jsonl"));
        let mut body = String::new();
        for i in 0..2000 {
            let mut record: Value = serde_json::from_str(&synthetic_line(i, SID)).unwrap();
            record["entrypoint"] = json!("cli");
            body += &(record.to_string() + "\n");
        }
        body += &(json!({"type":"assistant", "source_surface":surface, "entrypoint":"sdk"})
            .to_string()
            + "\n");
        fs::write(&file, body).unwrap();
        let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
        assert!(!run(&mut store, &home).complete());
        assert_eq!(store.counts().unwrap().records, 0);
        assert_eq!(
            store.discovered_sessions(Host::Claude).unwrap()[0].surface,
            None
        );
        let mut corrected: Value = serde_json::from_str(&synthetic_line(0, SID)).unwrap();
        corrected["entrypoint"] = json!("sdk");
        fs::write(&file, corrected.to_string() + "\n").unwrap();
        assert!(run(&mut store, &home).complete());
    }
}
