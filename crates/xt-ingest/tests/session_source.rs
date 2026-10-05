//! Opening one Claude session's original local source on demand.
//!
//! Every transcript here is written by the test: short synthetic lines in the
//! canonical shape, never a real conversation. The suite proves what a detail
//! view depends on — the source that loads is the session's own and is the
//! bytes that were verified, an unavailable or uncertain source says which way
//! it is unavailable, a cancelled read yields nothing, ceilings count over the
//! whole session, and opening a session writes nothing anywhere. Codex and
//! Cursor are refused before anything is started or written.
#![cfg(unix)]
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use xt_fixtures::TempDb;
use xt_ingest::native::{
    ImportRequest, ProducerSource, import_native,
    readers_cli::{CancelToken, read_pin},
    session_source::{
        GenerationBasis, IndexedSource, LoadedSession, SessionSourceOutcome, SessionSourceRequest,
        SourceCeiling, SourceGap, SourceLimits, SourceRole, SourceUnavailable, indexed_sources,
        load_session_source,
    },
};
use xt_store::{Host, Store};

const OBSERVED_AT: i64 = 1_788_782_400_000;
const SESSION: &str = "00000000-0000-4000-8000-00000000aaaa";
const OTHER: &str = "00000000-0000-4000-8000-00000000bbbb";
const PROJECT: &str = "-repo-fixture";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A distinct instant per turn, valid however many turns a fixture has.
fn stamp(index: usize) -> String {
    let (hour, minute, second) = (12 + index / 3600, (index / 60) % 60, index % 60);
    format!("2026-09-07T{hour:02}:{minute:02}:{second:02}.000Z")
}

/// One synthetic turn. Assistant turns carry a tool call, because the timeline
/// this loader feeds jumps to a stretch's first `tool_use` block.
fn turn(session: &str, index: usize) -> String {
    said(session, index, "ask")
}

/// One synthetic turn whose user text says which generation of a file it came
/// from, so a read that mixed two of them can be seen to have done so.
fn said(session: &str, index: usize, text: &str) -> String {
    let uuid = format!("11111111-1111-4111-8111-{index:012}");
    if index.is_multiple_of(2) {
        json!({
            "uuid": uuid, "type": "user", "sessionId": session, "cwd": "/repo/fixture",
            "timestamp": stamp(index),
            "message": {"role": "user", "content": [{"type": "text", "text": text}]}
        })
    } else {
        json!({
            "uuid": uuid, "type": "assistant", "sessionId": session, "cwd": "/repo/fixture",
            "timestamp": stamp(index),
            "message": {"role": "assistant", "model": "claude-fixture", "content": [
                {"type": "text", "text": "answer"},
                {"type": "tool_use", "id": format!("toolu_{index:04}"), "name": "Edit",
                 "input": {"file_path": "/repo/fixture/a.txt"}}
            ]}
        })
    }
    .to_string()
}

fn transcript(session: &str, turns: usize) -> String {
    said_transcript(session, turns, "ask")
}

fn said_transcript(session: &str, turns: usize, text: &str) -> String {
    (0..turns)
        .map(|index| format!("{}\n", said(session, index, text)))
        .collect()
}

/// A home holding one Claude project with one session's transcript.
fn home_with(root: &Path, project: &str, session: &str, body: &str) -> PathBuf {
    let home = root.join("home");
    let directory = home.join(".claude/projects").join(project);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join(format!("{session}.jsonl")), body).unwrap();
    home
}

fn transcript_path(home: &Path, project: &str, session: &str) -> PathBuf {
    home.join(".claude/projects")
        .join(project)
        .join(format!("{session}.jsonl"))
}

fn subagents(home: &Path, project: &str, session: &str) -> PathBuf {
    home.join(".claude/projects")
        .join(project)
        .join(session)
        .join("subagents")
}

/// Index the home exactly as the app does, so every later read is judged
/// against measurements that really were taken from these bytes.
fn index(store: &mut Store, home: &Path) {
    let producer = ProducerSource::Bundle {
        pin: read_pin(&repo().join(".plugin-pin")).unwrap(),
        root: repo().join("vendor/agent-plugins"),
    };
    let report = import_native(
        store,
        &ImportRequest {
            home,
            hosts: &[Host::Claude],
            producer: &producer,
            python: None,
            observed_at: OBSERVED_AT,
            cancel: None,
        },
    );
    assert!(report.complete(), "{report:?}");
}

fn open(
    home: &Path,
    session: &str,
    indexed: &[IndexedSource],
    cancel: Option<&CancelToken>,
    limits: SourceLimits,
) -> SessionSourceOutcome {
    load_session_source(&SessionSourceRequest {
        home,
        host: Host::Claude,
        native_session_id: session,
        indexed,
        cancel,
        limits,
    })
}

fn loaded(outcome: SessionSourceOutcome) -> LoadedSession {
    match outcome {
        SessionSourceOutcome::Loaded(session) => *session,
        SessionSourceOutcome::Unavailable(reason) => {
            panic!("expected the session's source, got {reason:?}")
        }
    }
}

fn unavailable(outcome: SessionSourceOutcome) -> SourceUnavailable {
    match outcome {
        SessionSourceOutcome::Unavailable(reason) => reason,
        SessionSourceOutcome::Loaded(session) => {
            panic!("expected no source, got {} records", session.records.len())
        }
    }
}

/// Every identity a loaded record carries.
fn identities(session: &LoadedSession) -> Vec<&str> {
    session
        .records
        .iter()
        .flat_map(|record| {
            [
                record.native.session_id.as_deref(),
                record.canonical.native_session_id.as_deref(),
            ]
        })
        .flatten()
        .collect()
}

/// Every path under a root with its size, so a read that left something behind
/// would show up as a difference.
fn tree(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                pending.push(path.clone());
            }
            found.push((path, meta.len()));
        }
    }
    found.sort();
    found
}

#[test]
fn the_session_loads_its_own_transcript_in_the_generation_the_index_measured() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 6));
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    assert_eq!(
        indexed.len(),
        1,
        "the index recorded this session's locator"
    );
    let session = loaded(open(
        &home,
        SESSION,
        &indexed,
        None,
        SourceLimits::default(),
    ));
    assert_eq!(session.native_session_id, SESSION);
    assert_eq!(session.records.len(), 6);
    assert_eq!(
        session.generation,
        GenerationBasis::Indexed { appended: false }
    );
    assert_eq!(session.gaps, vec![]);
    assert_eq!(session.sources.len(), 1);
    assert_eq!(session.sources[0].role, SourceRole::Primary);
    assert_eq!(
        session.sources[0].path,
        transcript_path(&home, PROJECT, SESSION)
    );
    // The jump target U-09 needs is present in what was loaded.
    assert!(session.records.iter().any(|record| {
        record
            .canonical
            .message
            .content
            .iter()
            .flatten()
            .any(|block| block["type"] == "tool_use" && block["id"] == "toolu_0001")
    }));
    assert!(identities(&session).iter().all(|id| *id == SESSION));
}

#[test]
fn an_append_completed_before_the_open_is_read_whole_and_reported_as_growth() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    let path = transcript_path(&home, PROJECT, SESSION);
    let mut grown = fs::read_to_string(&path).unwrap();
    grown.push_str(&turn(SESSION, 4));
    grown.push('\n');
    fs::write(&path, grown).unwrap();
    // The append finished before the read began: it is part of the length the
    // read verifies, and the checkpoint says the session has grown.
    let session = loaded(open(
        &home,
        SESSION,
        &indexed,
        None,
        SourceLimits::default(),
    ));
    assert_eq!(
        session.generation,
        GenerationBasis::Indexed { appended: true }
    );
    assert_eq!(session.records.len(), 5);
    assert_eq!(session.gaps, vec![]);
}

#[test]
fn a_session_with_no_file_under_the_home_is_missing() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, OTHER, &transcript(OTHER, 2));
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    assert!(indexed.is_empty());
    assert_eq!(
        unavailable(open(
            &home,
            SESSION,
            &indexed,
            None,
            SourceLimits::default()
        )),
        SourceUnavailable::Missing
    );
    // A home with no Claude history at all is missing, not an error.
    let empty = temp.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    assert_eq!(
        unavailable(open(&empty, SESSION, &[], None, SourceLimits::default())),
        SourceUnavailable::Missing
    );
}

#[test]
fn a_project_that_cannot_be_looked_through_is_never_reported_as_an_absent_session() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, OTHER, &transcript(OTHER, 2));
    let closed = home.join(".claude/projects/-repo-closed");
    fs::create_dir_all(&closed).unwrap();
    fs::write(
        closed.join(format!("{SESSION}.jsonl")),
        transcript(SESSION, 2),
    )
    .unwrap();
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).unwrap();
    // The session could be in the directory that cannot be listed, so an
    // absent session is not a claim this read is entitled to make.
    let reason = unavailable(open(&home, SESSION, &[], None, SourceLimits::default()));
    assert!(
        matches!(reason, SourceUnavailable::Unreadable { .. }),
        "{reason:?}"
    );
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn a_session_found_beside_a_project_that_could_not_be_listed_is_not_shown_as_the_only_one() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let closed = home.join(".claude/projects/-repo-closed");
    fs::create_dir_all(&closed).unwrap();
    // A second file of the same name is really in there. Whether it is or not
    // cannot be seen, which is the point: a candidate beside a place that
    // could hold its twin is not known to be the only one, and showing it
    // would be the ambiguity rule with the ambiguity merely unseen.
    fs::write(
        closed.join(format!("{SESSION}.jsonl")),
        transcript(SESSION, 9),
    )
    .unwrap();
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).unwrap();
    let reason = unavailable(open(&home, SESSION, &[], None, SourceLimits::default()));
    assert!(
        matches!(reason, SourceUnavailable::Unreadable { .. }),
        "{reason:?}"
    );
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn a_subagent_transcript_the_index_read_and_that_is_now_gone_is_named_as_missing() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let agents = subagents(&home, PROJECT, SESSION);
    fs::create_dir_all(agents.join("kept")).unwrap();
    fs::create_dir_all(agents.join("gone")).unwrap();
    fs::write(agents.join("kept/log.jsonl"), transcript(SESSION, 2)).unwrap();
    fs::write(agents.join("gone/log.jsonl"), transcript(SESSION, 3)).unwrap();
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    assert_eq!(indexed.len(), 3);
    let before = db.store().counts().unwrap();
    // The measurements still count what that file said, so a session without
    // it is not the session those numbers describe.
    fs::remove_file(agents.join("gone/log.jsonl")).unwrap();
    let session = loaded(open(
        &home,
        SESSION,
        &indexed,
        None,
        SourceLimits::default(),
    ));
    assert_eq!(
        session.sources.len(),
        2,
        "the transcript and the kept agent"
    );
    assert_eq!(
        session
            .gaps
            .iter()
            .filter(|gap| matches!(gap, SourceGap::Missing { .. }))
            .count(),
        1,
        "{:?}",
        session.gaps
    );
    assert_eq!(db.store().counts().unwrap(), before, "nothing was written");
}

#[test]
fn a_session_whose_files_moved_together_is_not_mistaken_for_one_that_lost_a_file() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let agents = subagents(&home, PROJECT, SESSION);
    fs::create_dir_all(agents.join("a")).unwrap();
    fs::write(agents.join("a/log.jsonl"), transcript(SESSION, 2)).unwrap();
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    // The whole project directory is renamed and read again, so the index
    // holds both spellings of every file. The old spellings name files that
    // are not there under that name any more, but no file was lost.
    let renamed = home.join(".claude/projects/-repo-renamed");
    fs::rename(home.join(".claude/projects").join(PROJECT), &renamed).unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    assert_eq!(indexed.len(), 4, "both spellings of both files");
    let session = loaded(open(
        &home,
        SESSION,
        &indexed,
        None,
        SourceLimits::default(),
    ));
    assert_eq!(session.sources.len(), 2);
    assert!(
        !session
            .gaps
            .iter()
            .any(|gap| matches!(gap, SourceGap::Missing { .. })),
        "{:?}",
        session.gaps
    );
}

#[test]
fn subagent_files_sharing_a_name_keep_their_own_checkpoints() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let agents = subagents(&home, PROJECT, SESSION);
    // Two subagent transcripts whose file names are identical: each must be
    // measured against its own recorded generation, not its namesake's.
    for (agent, turns) in [("a", 2), ("b", 3)] {
        fs::create_dir_all(agents.join(agent)).unwrap();
        fs::write(
            agents.join(agent).join("log.jsonl"),
            transcript(SESSION, turns),
        )
        .unwrap();
    }
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    assert_eq!(indexed.len(), 3);
    let session = loaded(open(
        &home,
        SESSION,
        &indexed,
        None,
        SourceLimits::default(),
    ));
    assert_eq!(session.sources.len(), 3);
    assert_eq!(session.gaps, vec![]);
    assert_eq!(
        session.generation,
        GenerationBasis::Indexed { appended: false }
    );
}

#[test]
fn a_transcript_that_moved_since_it_was_indexed_is_named_moved_and_is_not_read() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    let from = transcript_path(&home, PROJECT, SESSION);
    let moved_project = home.join(".claude/projects/-repo-renamed");
    fs::create_dir_all(&moved_project).unwrap();
    let to = moved_project.join(format!("{SESSION}.jsonl"));
    fs::rename(&from, &to).unwrap();
    assert_eq!(
        unavailable(open(
            &home,
            SESSION,
            &indexed,
            None,
            SourceLimits::default()
        )),
        SourceUnavailable::Moved {
            from: from.clone(),
            found: to.clone()
        }
    );
    // Once the index has read it where it now is, it is no longer moved.
    index(db.store_mut(), &home);
    let reindexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    assert_eq!(reindexed.len(), 2, "both spellings are recorded");
    let session = loaded(open(
        &home,
        SESSION,
        &reindexed,
        None,
        SourceLimits::default(),
    ));
    assert_eq!(session.sources[0].path, to);
}

#[test]
fn a_transcript_that_cannot_be_opened_is_unreadable_and_keeps_its_metrics() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    let before = db.store().counts().unwrap();
    let path = transcript_path(&home, PROJECT, SESSION);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let reason = unavailable(open(
        &home,
        SESSION,
        &indexed,
        None,
        SourceLimits::default(),
    ));
    assert!(
        matches!(reason, SourceUnavailable::Unreadable { .. }),
        "{reason:?}"
    );
    // The measurements taken when it was readable are untouched.
    assert_eq!(db.store().counts().unwrap(), before);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn a_source_replaced_truncated_or_rewritten_since_indexing_shows_no_text() {
    for change in ["replace", "truncate", "rewrite"] {
        let temp = tempfile::TempDir::new().unwrap();
        let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 6));
        let mut db = TempDb::empty().unwrap();
        index(db.store_mut(), &home);
        let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
        let path = transcript_path(&home, PROJECT, SESSION);
        match change {
            // A new inode at the same name.
            "replace" => {
                let staged = temp.path().join("staged.jsonl");
                fs::write(&staged, transcript(SESSION, 3)).unwrap();
                fs::rename(&staged, &path).unwrap();
            }
            "truncate" => fs::write(&path, transcript(SESSION, 2)).unwrap(),
            // Same length, different bytes before the recorded position.
            _ => {
                let rewritten = transcript(SESSION, 6).replace("\"ask\"", "\"AsK\"");
                assert_eq!(rewritten.len(), transcript(SESSION, 6).len());
                fs::write(&path, rewritten).unwrap();
            }
        }
        let reason = unavailable(open(
            &home,
            SESSION,
            &indexed,
            None,
            SourceLimits::default(),
        ));
        assert!(
            matches!(reason, SourceUnavailable::Replaced { .. }),
            "{change}: {reason:?}"
        );
    }
}

#[test]
fn two_files_naming_one_session_show_neither() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let second = home.join(".claude/projects/-repo-copy");
    fs::create_dir_all(&second).unwrap();
    fs::write(
        second.join(format!("{SESSION}.jsonl")),
        transcript(SESSION, 9),
    )
    .unwrap();
    assert_eq!(
        unavailable(open(&home, SESSION, &[], None, SourceLimits::default())),
        SourceUnavailable::Ambiguous { candidates: 2 }
    );
}

#[test]
fn a_transcript_whose_records_name_another_session_is_that_files_session_only() {
    let temp = tempfile::TempDir::new().unwrap();
    // A fork carries its original's `sessionId` in the prefix it inherited.
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(OTHER, 4));
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    let session = loaded(open(
        &home,
        SESSION,
        &indexed,
        None,
        SourceLimits::default(),
    ));
    assert_eq!(session.records.len(), 4);
    assert!(
        identities(&session).iter().all(|id| *id == SESSION),
        "the file the session owns settles its identity"
    );
    // Asking for the session the copied prefix names must not reach this file.
    assert_eq!(
        unavailable(open(&home, OTHER, &[], None, SourceLimits::default())),
        SourceUnavailable::Missing
    );
}

#[test]
fn a_record_whose_own_identity_labels_disagree_stops_the_file_where_the_importer_stops() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut body = transcript(SESSION, 2);
    body.push_str(&format!(
        "{}\n",
        json!({
            "uuid": "22222222-2222-4222-8222-222222222222", "type": "user",
            "sessionId": SESSION, "native_session_id": OTHER,
            "timestamp": "2026-09-07T12:30:00.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "ask"}]}
        })
    ));
    body.push_str(&format!("{}\n", turn(SESSION, 3)));
    let home = home_with(temp.path(), PROJECT, SESSION, &body);
    let session = loaded(open(&home, SESSION, &[], None, SourceLimits::default()));
    assert_eq!(session.records.len(), 2, "the lines before the bad one");
    assert!(
        matches!(
            session.gaps.as_slice(),
            [SourceGap::Stopped { line: 3, .. }]
        ),
        "{:?}",
        session.gaps
    );
}

#[test]
fn a_cancelled_read_returns_no_content_at_all() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 40));
    let cancel = CancelToken::new();
    cancel.cancel();
    assert_eq!(
        unavailable(open(
            &home,
            SESSION,
            &[],
            Some(&cancel),
            SourceLimits::default()
        )),
        SourceUnavailable::Cancelled
    );
}

#[test]
fn ceilings_count_over_the_whole_session_and_report_their_own_unit() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 20));
    let bytes = SourceLimits {
        max_bytes: 64,
        max_records: 1_000,
    };
    assert!(matches!(
        unavailable(open(&home, SESSION, &[], None, bytes)),
        SourceUnavailable::TooLarge {
            limit: SourceCeiling::Bytes,
            ceiling: 64,
            ..
        }
    ));
    // The ceiling that stopped the read is the one that is reported: a record
    // ceiling counts records, never bytes.
    let records = SourceLimits {
        max_bytes: 1 << 20,
        max_records: 5,
    };
    assert!(matches!(
        unavailable(open(&home, SESSION, &[], None, records)),
        SourceUnavailable::TooLarge {
            limit: SourceCeiling::Records,
            reached: 6,
            ceiling: 5,
        }
    ));
}

#[test]
fn a_subagent_file_past_the_ceiling_makes_the_whole_session_too_large() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let agents = subagents(&home, PROJECT, SESSION);
    fs::create_dir_all(&agents).unwrap();
    fs::write(agents.join("agent.jsonl"), transcript(SESSION, 40)).unwrap();
    let primary = fs::symlink_metadata(transcript_path(&home, PROJECT, SESSION))
        .unwrap()
        .len();
    // Room for the session's own transcript, not for its subagent file: the
    // session, not the file that happened to reach the ceiling, is too large.
    let limits = SourceLimits {
        max_bytes: primary + 16,
        max_records: 1_000,
    };
    assert!(matches!(
        unavailable(open(&home, SESSION, &[], None, limits)),
        SourceUnavailable::TooLarge {
            limit: SourceCeiling::Bytes,
            ..
        }
    ));
}

#[test]
fn an_identifier_that_could_name_something_else_is_refused_before_any_lookup() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 2));
    for identifier in [
        "",
        "latest",
        "..",
        ".hidden",
        "../../etc/passwd",
        "a/b",
        "a\\b",
        " spaced ",
        "line\nbreak",
    ] {
        assert_eq!(
            unavailable(open(&home, identifier, &[], None, SourceLimits::default())),
            SourceUnavailable::InvalidIdentifier,
            "{identifier:?}"
        );
    }
}

#[test]
fn subagent_files_of_the_session_load_after_its_own_transcript() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let agents = subagents(&home, PROJECT, SESSION);
    fs::create_dir_all(&agents).unwrap();
    fs::write(
        agents.join("agent.jsonl"),
        format!(
            "{}\n",
            json!({
                "uuid": "33333333-3333-4333-8333-333333333333", "type": "assistant",
                "sessionId": SESSION, "isSidechain": true,
                "timestamp": "2026-09-07T12:40:00.000Z",
                "message": {"role": "assistant", "content": [{"type": "text", "text": "sub"}]}
            })
        ),
    )
    .unwrap();
    let session = loaded(open(&home, SESSION, &[], None, SourceLimits::default()));
    assert_eq!(session.records.len(), 5);
    assert_eq!(session.sources.len(), 2);
    assert_eq!(session.sources[1].role, SourceRole::Sidechain);
    assert!(session.records.last().unwrap().canonical.is_sidechain);
    assert!(identities(&session).iter().all(|id| *id == SESSION));
}

#[test]
fn a_subagent_file_the_index_never_read_leaves_the_whole_session_untied() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let mut db = TempDb::empty().unwrap();
    // The index reads the session's own transcript, and only then does a
    // subagent file appear: the session as a whole can no longer be tied to
    // what was measured, even though its transcript can.
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    assert_eq!(indexed.len(), 1);
    let agents = subagents(&home, PROJECT, SESSION);
    fs::create_dir_all(&agents).unwrap();
    fs::write(agents.join("agent.jsonl"), transcript(SESSION, 2)).unwrap();
    let session = loaded(open(
        &home,
        SESSION,
        &indexed,
        None,
        SourceLimits::default(),
    ));
    assert_eq!(session.sources.len(), 2);
    assert_eq!(session.generation, GenerationBasis::Unrecorded);

    // Once the index has read both, a later append to the subagent file alone
    // makes the whole session a grown one.
    index(db.store_mut(), &home);
    let reindexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    assert_eq!(reindexed.len(), 2);
    let mut grown = fs::read_to_string(agents.join("agent.jsonl")).unwrap();
    grown.push_str(&turn(SESSION, 9));
    grown.push('\n');
    fs::write(agents.join("agent.jsonl"), grown).unwrap();
    let session = loaded(open(
        &home,
        SESSION,
        &reindexed,
        None,
        SourceLimits::default(),
    ));
    assert_eq!(
        session.generation,
        GenerationBasis::Indexed { appended: true }
    );
}

#[test]
fn an_unreadable_subagent_file_is_a_named_gap_not_a_silent_omission() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let agents = subagents(&home, PROJECT, SESSION);
    fs::create_dir_all(&agents).unwrap();
    let file = agents.join("agent.jsonl");
    fs::write(&file, transcript(SESSION, 2)).unwrap();
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    assert_eq!(indexed.len(), 2);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    let session = loaded(open(
        &home,
        SESSION,
        &indexed,
        None,
        SourceLimits::default(),
    ));
    assert_eq!(session.records.len(), 4);
    // One cause, one gap: a file that is there and cannot be read is not also
    // reported as a file that is not there.
    assert!(
        matches!(session.gaps.as_slice(), [SourceGap::Unreadable { .. }]),
        "{:?}",
        session.gaps
    );
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn a_subagent_directory_that_cannot_be_listed_says_the_session_may_be_incomplete() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    let agents = subagents(&home, PROJECT, SESSION);
    fs::create_dir_all(&agents).unwrap();
    fs::write(agents.join("agent.jsonl"), transcript(SESSION, 2)).unwrap();
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    assert_eq!(indexed.len(), 2);
    fs::set_permissions(&agents, fs::Permissions::from_mode(0o000)).unwrap();
    let session = loaded(open(
        &home,
        SESSION,
        &indexed,
        None,
        SourceLimits::default(),
    ));
    assert_eq!(session.records.len(), 4, "its own transcript still loads");
    assert!(
        session.gaps.contains(&SourceGap::DiscoveryIncomplete),
        "{:?}",
        session.gaps
    );
    // The file the index read is still there; it simply could not be listed.
    // Calling it lost would claim more than this read managed to see.
    assert!(
        !session
            .gaps
            .iter()
            .any(|gap| matches!(gap, SourceGap::Missing { .. })),
        "{:?}",
        session.gaps
    );
    fs::set_permissions(&agents, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn an_alias_in_place_of_a_transcript_is_never_followed() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, OTHER, &transcript(OTHER, 4));
    let target = transcript_path(&home, PROJECT, OTHER);
    let alias = transcript_path(&home, PROJECT, SESSION);
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    let reason = unavailable(open(&home, SESSION, &[], None, SourceLimits::default()));
    // The alias is not a discovered transcript, and it is not silently absent
    // either: something named this session and could not be used.
    assert!(
        matches!(reason, SourceUnavailable::Unreadable { .. }),
        "{reason:?}"
    );
}

#[test]
fn an_alias_in_place_of_the_session_directory_never_files_a_foreign_tree_under_it() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 4));
    // Another session's subagent tree, reachable only through an alias placed
    // where this session's own directory would be.
    let foreign = home
        .join(".claude/projects")
        .join(PROJECT)
        .join(OTHER)
        .join("subagents");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("agent.jsonl"), transcript(OTHER, 3)).unwrap();
    std::os::unix::fs::symlink(
        home.join(".claude/projects").join(PROJECT).join(OTHER),
        home.join(".claude/projects").join(PROJECT).join(SESSION),
    )
    .unwrap();
    let session = loaded(open(&home, SESSION, &[], None, SourceLimits::default()));
    assert_eq!(
        session.records.len(),
        4,
        "only the session's own transcript was read"
    );
    assert_eq!(session.sources.len(), 1);
    assert!(
        session.gaps.contains(&SourceGap::DiscoveryIncomplete),
        "an alias where a session's directory belongs is not passed over: {:?}",
        session.gaps
    );
}

#[test]
fn opening_a_session_writes_nothing_and_changes_no_source_byte() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 8));
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    let path = transcript_path(&home, PROJECT, SESSION);
    let before_bytes = fs::read(&path).unwrap();
    let before_meta = fs::symlink_metadata(&path).unwrap();
    let before_counts = db.store().counts().unwrap();
    let before_tree = tree(&home);
    let session = loaded(open(
        &home,
        SESSION,
        &indexed,
        None,
        SourceLimits::default(),
    ));
    assert_eq!(session.records.len(), 8);
    assert_eq!(fs::read(&path).unwrap(), before_bytes);
    let after_meta = fs::symlink_metadata(&path).unwrap();
    assert_eq!(after_meta.len(), before_meta.len());
    assert_eq!(
        after_meta.modified().unwrap(),
        before_meta.modified().unwrap()
    );
    // No new file anywhere under the home: no cache, no copy, no export.
    assert_eq!(tree(&home), before_tree);
    // No row, session or usage observation was added by viewing.
    assert_eq!(db.store().counts().unwrap(), before_counts);
    // Dropping the loaded session is the whole of its lifecycle.
    drop(session);
    assert_eq!(db.store().counts().unwrap(), before_counts);
}

#[test]
fn a_source_replaced_between_two_reads_stops_being_shown() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(temp.path(), PROJECT, SESSION, &transcript(SESSION, 6));
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_sources(db.store(), Host::Claude, SESSION).unwrap();
    assert_eq!(
        loaded(open(
            &home,
            SESSION,
            &indexed,
            None,
            SourceLimits::default()
        ))
        .records
        .len(),
        6
    );
    let staged = temp.path().join("staged.jsonl");
    fs::write(&staged, transcript(SESSION, 2)).unwrap();
    fs::rename(&staged, transcript_path(&home, PROJECT, SESSION)).unwrap();
    assert!(matches!(
        unavailable(open(
            &home,
            SESSION,
            &indexed,
            None,
            SourceLimits::default()
        )),
        SourceUnavailable::Replaced { .. }
    ));
}

/// Whether the records are the fixture's own sequence, unbroken from its
/// start. A read that spliced two generations together, or stopped short
/// inside one, would break it.
fn unbroken(session: &LoadedSession) -> bool {
    session.records.iter().enumerate().all(|(index, record)| {
        record.canonical.uuid.as_deref() == Some(&format!("11111111-1111-4111-8111-{index:012}"))
    })
}

/// The distinct text this session's user turns carry. Identifiers alone cannot
/// tell two generations of one file apart when only the words changed, so the
/// words are what a settled snapshot is judged by: one generation, one answer.
fn said_by(session: &LoadedSession) -> BTreeSet<String> {
    session
        .records
        .iter()
        .filter(|record| record.canonical.message.role.as_deref() == Some("user"))
        .filter_map(|record| {
            record.canonical.message.content.as_ref()?.first()?["text"]
                .as_str()
                .map(str::to_owned)
        })
        .collect()
}

/// A source big enough that a writer can land inside the read of it. Whether
/// one does is timing, and the refusal of a file that changed inside its own
/// read is proven by construction in the crate's unit tests instead; these
/// cases assert the property that must hold however the timing falls.
const CONTENDED_TURNS: usize = 10_000;

#[test]
fn a_source_replaced_while_it_is_being_read_never_yields_mixed_content() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(
        temp.path(),
        PROJECT,
        SESSION,
        &transcript(SESSION, CONTENDED_TURNS),
    );
    let path = transcript_path(&home, PROJECT, SESSION);
    let replacement = temp.path().join("replacement.jsonl");
    let mut refused = 0;
    for attempt in 0..8 {
        fs::write(&path, transcript(SESSION, CONTENDED_TURNS)).unwrap();
        fs::write(&replacement, said_transcript(SESSION, 7, "swapped")).unwrap();
        let swapping = {
            let (path, replacement) = (path.clone(), replacement.clone());
            std::thread::spawn(move || {
                let _ = fs::rename(&replacement, &path);
            })
        };
        match open(&home, SESSION, &[], None, SourceLimits::default()) {
            // Whichever generation was opened, it is read whole and named as
            // this session: a swap never splices two files together.
            SessionSourceOutcome::Loaded(session) => {
                // One generation, whole, and every word in it from that one
                // generation: the two say different things, so a read that
                // mixed them would carry both.
                let said = said_by(&session);
                assert!(
                    said == BTreeSet::from(["ask".to_owned()])
                        && session.records.len() == CONTENDED_TURNS
                        || said == BTreeSet::from(["swapped".to_owned()])
                            && session.records.len() == 7,
                    "attempt {attempt} read {} records saying {said:?}",
                    session.records.len()
                );
                assert!(unbroken(&session), "attempt {attempt} spliced its records");
                assert!(identities(&session).iter().all(|id| *id == SESSION));
                assert_eq!(session.gaps, vec![]);
            }
            SessionSourceOutcome::Unavailable(SourceUnavailable::Replaced { .. }) => refused += 1,
            other => panic!("attempt {attempt}: {other:?}"),
        }
        swapping.join().unwrap();
    }
    println!("mid-read replacements refused: {refused}");
}

#[test]
fn a_source_appended_to_while_it_is_being_read_is_refused_rather_than_shown() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with(
        temp.path(),
        PROJECT,
        SESSION,
        &transcript(SESSION, CONTENDED_TURNS),
    );
    let path = transcript_path(&home, PROJECT, SESSION);
    let mut refused = 0;
    for attempt in 0..8 {
        fs::write(&path, transcript(SESSION, CONTENDED_TURNS)).unwrap();
        let appending = {
            let path = path.clone();
            // The appended turns continue the fixture's own sequence, so a
            // read that caught some of them is still unbroken, and only a
            // spliced or short read could break it.
            std::thread::spawn(move || {
                if let Ok(mut file) = fs::OpenOptions::new().append(true).open(&path) {
                    for index in CONTENDED_TURNS..CONTENDED_TURNS + 40 {
                        let line = format!("{}\n", turn(SESSION, index));
                        if std::io::Write::write_all(&mut file, line.as_bytes()).is_err() {
                            return;
                        }
                    }
                }
            })
        };
        match open(&home, SESSION, &[], None, SourceLimits::default()) {
            // An append that landed before the open is part of what is read;
            // one that lands during the read is a change, and a changed source
            // is refused rather than shown as a settled transcript.
            SessionSourceOutcome::Loaded(session) => {
                assert!(
                    session.records.len() >= CONTENDED_TURNS,
                    "attempt {attempt} read {} records",
                    session.records.len()
                );
                assert_eq!(
                    said_by(&session),
                    BTreeSet::from(["ask".to_owned()]),
                    "attempt {attempt} carried words from another generation"
                );
                assert!(unbroken(&session), "attempt {attempt} spliced its records");
                assert_eq!(session.gaps, vec![]);
            }
            SessionSourceOutcome::Unavailable(SourceUnavailable::Replaced { .. }) => refused += 1,
            other => panic!("attempt {attempt}: {other:?}"),
        }
        appending.join().unwrap();
    }
    println!("mid-read appends refused: {refused}");
}

#[test]
fn a_source_rewritten_in_place_while_it_is_being_read_never_yields_mixed_content() {
    let temp = tempfile::TempDir::new().unwrap();
    let settled = transcript(SESSION, CONTENDED_TURNS);
    // The same length, different words: nothing is renamed and nothing grows,
    // so only the change time and the words themselves tell the two apart.
    let rewritten = said_transcript(SESSION, CONTENDED_TURNS, "AsK");
    assert_eq!(settled.len(), rewritten.len());
    let home = home_with(temp.path(), PROJECT, SESSION, &settled);
    let path = transcript_path(&home, PROJECT, SESSION);
    let mut refused = 0;
    for attempt in 0..8 {
        fs::write(&path, &settled).unwrap();
        let writing = {
            let (path, rewritten) = (path.clone(), rewritten.clone());
            std::thread::spawn(move || {
                if let Ok(mut file) = fs::OpenOptions::new().write(true).open(&path) {
                    let _ = std::io::Write::write_all(&mut file, rewritten.as_bytes());
                }
            })
        };
        match open(&home, SESSION, &[], None, SourceLimits::default()) {
            // Whether the writer lands inside this read is timing; that a file
            // changed inside its own read is refused is proven by construction
            // in the crate's unit tests. What must hold however it falls is
            // that a read which was not refused carries one generation's words
            // and not a line of the other's.
            SessionSourceOutcome::Loaded(session) => {
                let said = said_by(&session);
                assert!(
                    said == BTreeSet::from(["ask".to_owned()])
                        || said == BTreeSet::from(["AsK".to_owned()]),
                    "attempt {attempt} read words from both generations: {said:?}"
                );
                assert_eq!(session.records.len(), CONTENDED_TURNS);
                assert!(unbroken(&session), "attempt {attempt} spliced its records");
                assert_eq!(session.gaps, vec![]);
            }
            SessionSourceOutcome::Unavailable(SourceUnavailable::Replaced { .. }) => refused += 1,
            other => panic!("attempt {attempt}: {other:?}"),
        }
        writing.join().unwrap();
    }
    println!("mid-read rewrites refused: {refused}");
}

#[test]
fn codex_and_cursor_are_refused_without_starting_anything_or_writing_a_file() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    // History that a reader would have something to do with, if one ran.
    for (host, relative) in [
        ("codex", ".codex/sessions/2026/09/07"),
        ("cursor", format!(".cursor/chats/{SESSION}").as_str()),
    ] {
        let directory = home.join(relative);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(format!("{host}-{SESSION}.jsonl")),
            transcript(SESSION, 2),
        )
        .unwrap();
    }
    let before = tree(&home);
    for host in [Host::Codex, Host::Cursor] {
        let outcome = load_session_source(&SessionSourceRequest {
            home: &home,
            host,
            native_session_id: SESSION,
            indexed: &[],
            cancel: None,
            limits: SourceLimits::default(),
        });
        assert_eq!(
            unavailable(outcome),
            SourceUnavailable::PrerequisiteUnavailable,
            "{host:?}"
        );
    }
    // The request carries no interpreter and no producer, so there is nothing
    // to start; nothing was staged, copied or left behind either.
    assert_eq!(tree(&home), before);
    assert_eq!(
        unavailable(load_session_source(&SessionSourceRequest {
            home: &home,
            host: Host::Other,
            native_session_id: SESSION,
            indexed: &[],
            cancel: None,
            limits: SourceLimits::default(),
        })),
        SourceUnavailable::UnsupportedHost
    );
}
