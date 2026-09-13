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
    import_native(
        store,
        &ImportRequest {
            home,
            hosts: &[Host::Claude],
            pin: &repo().join(".plugin-pin"),
            plugin_root: None,
            python: None,
            observed_at: 1_788_782_400_000,
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
}
