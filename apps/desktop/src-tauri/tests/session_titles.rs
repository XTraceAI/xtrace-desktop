//! Host titles for visible rows, through the app's own command path.
//!
//! The view names rows by canonical identifier; the app resolves each against
//! the index, reads the recorded source's title and returns it for exactly
//! the rows asked about. The list itself — membership, order, pages, search —
//! and everything stored are unchanged by it.
//!
//! Every transcript here is written by the test: short synthetic lines, never
//! a real conversation.
#![cfg(unix)]
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use xt_ingest::native::{
    ImportRequest, ProducerSource, import_native,
    readers_cli::{CancelToken, read_pin},
};
use xt_store::{Host, Store};
use xtrace_desktop::{
    dto::{SessionTitle, SessionTitles},
    state::{AppState, StartupOptions, StateError},
};

const OBSERVED_AT: i64 = 1_788_782_400_000;
const PROJECT: &str = "-repo-fixture";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn native(index: usize) -> String {
    format!("00000000-0000-4000-8000-{index:012}")
}

/// One user turn and one answer, then whatever title lines the test adds.
fn transcript(session: &str, titles: &[serde_json::Value]) -> String {
    let mut lines = vec![
        json!({
            "uuid": format!("{}-u", &session[..30]), "type": "user", "sessionId": session,
            "cwd": "/repo/fixture", "timestamp": "2026-09-07T12:00:00.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "ask something"}]}
        }),
        json!({
            "uuid": format!("{}-a", &session[..30]), "type": "assistant", "sessionId": session,
            "cwd": "/repo/fixture", "timestamp": "2026-09-07T12:00:01.000Z",
            "message": {"role": "assistant", "model": "claude-fixture",
                        "content": [{"type": "text", "text": "answer"}]}
        }),
    ];
    lines.extend(titles.iter().cloned());
    lines.iter().map(|line| format!("{line}\n")).collect()
}

fn write_session(home: &Path, session: &str, titles: &[serde_json::Value]) -> PathBuf {
    let path = home
        .join(".claude/projects")
        .join(PROJECT)
        .join(format!("{session}.jsonl"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, transcript(session, titles)).unwrap();
    path
}

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

fn titles(state: &AppState, ids: &[String]) -> SessionTitles {
    state.session_titles(ids, &CancelToken::new()).unwrap()
}

#[test]
fn compaction_command_returns_each_requested_id_without_changing_storage() {
    use xtrace_desktop::dto::{
        CompactionCount, CompactionEvent, CompactionReason, CompactionTrigger,
    };
    let directory = tempfile::Builder::new()
        .prefix("xtrace-compaction-command-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let home = directory.path().join("home");
    let native = native(21);
    let event = serde_json::json!({"type":"system","subtype":"compact_boundary","sessionId":native,"isSidechain":false,"uuid":"00000000-0000-4000-8000-000000000091","timestamp":"2026-09-07T12:00:02Z"});
    write_session(
        &home,
        &native,
        &[
            event.clone(),
            event,
            serde_json::json!({"type":"user","isCompactSummary":true,"sessionId":native,"uuid":"00000000-0000-4000-8000-000000000092","message":{"role":"user","content":"synthetic summary"}}),
        ],
    );
    let db = directory.path().join("data/xtrace.db");
    index(&db, &home);
    let state = state(directory.path());
    let before = state.db_counts().unwrap();
    let ids = vec![native.clone(), "not-indexed".to_owned()];
    let answer = state
        .session_compactions(&ids, &CancelToken::new())
        .unwrap();
    assert_eq!(
        answer
            .counts
            .iter()
            .map(|v| v.id.clone())
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(
        answer.counts[0].outcome,
        CompactionCount::Count {
            count: 1,
            // The repeated marker is counted once; it records no trigger.
            events: vec![CompactionEvent {
                at_ms: 1_788_782_402_000,
                trigger: CompactionTrigger::Unknown,
            }],
            inherited: None,
        }
    );
    assert_eq!(
        answer.counts[1].outcome,
        CompactionCount::Unknown {
            reason: CompactionReason::NotIndexed
        }
    );
    assert_eq!(state.db_counts().unwrap(), before);
    assert!(matches!(
        state.session_compactions(&[native.clone(), native.clone()], &CancelToken::new()),
        Err(StateError::InvalidCompactionRequest)
    ));
    assert!(matches!(
        state.session_compactions(
            &(0..51).map(|n| n.to_string()).collect::<Vec<_>>(),
            &CancelToken::new()
        ),
        Err(StateError::InvalidCompactionRequest)
    ));
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(
        state
            .session_compactions(&[native], &cancel)
            .unwrap()
            .counts
            .iter()
            .all(|row| matches!(
                row.outcome,
                CompactionCount::Unknown {
                    reason: CompactionReason::Cancelled
                }
            ))
    );
}

fn ai(session: &str, title: &str) -> serde_json::Value {
    json!({"type": "ai-title", "aiTitle": title, "sessionId": session})
}

fn rename(session: &str, title: &str) -> serde_json::Value {
    json!({"type": "custom-title", "customTitle": title, "sessionId": session})
}

/// A home of three indexed sessions: renamed after a generated title, titled
/// only by the host's generator, and untitled.
fn three_sessions(root: &Path) -> (AppState, [String; 3]) {
    let home = root.join("home");
    let [renamed, generated, untitled] = [native(1), native(2), native(3)];
    write_session(
        &home,
        &renamed,
        &[
            ai(&renamed, "Generated"),
            rename(&renamed, "Renamed"),
            ai(&renamed, "Stale generated"),
        ],
    );
    write_session(&home, &generated, &[ai(&generated, "Generated only")]);
    write_session(&home, &untitled, &[]);
    index(&root.join("data/xtrace.db"), &home);
    (state(root), [renamed, generated, untitled])
}

/// Exactly the rows asked about, in request order, and only those that have a
/// host title; an identifier the index does not hold is simply absent.
#[test]
fn titles_map_exactly_to_the_requested_rows() {
    let root = tempfile::tempdir().unwrap();
    let (state, [renamed, generated, untitled]) = three_sessions(root.path());
    let unknown = native(99);
    let answer = titles(
        &state,
        &[
            untitled.clone(),
            generated.clone(),
            unknown,
            renamed.clone(),
        ],
    );
    assert_eq!(
        answer.titles,
        [
            SessionTitle {
                id: generated.clone(),
                title: "Generated only".into()
            },
            SessionTitle {
                id: renamed.clone(),
                title: "Renamed".into()
            },
        ]
    );
    // A subset asks about a subset.
    assert_eq!(
        titles(&state, std::slice::from_ref(&renamed)).titles,
        [SessionTitle {
            id: renamed,
            title: "Renamed".into()
        }]
    );
    assert!(titles(&state, &[]).titles.is_empty());
}

/// More than one page, or the same row twice, is refused whole before the
/// index is consulted, rather than silently cut to fit.
#[test]
fn a_request_beyond_one_page_or_with_repeats_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (state, [renamed, ..]) = three_sessions(root.path());
    let page: Vec<String> = (0..50).map(native).collect();
    assert!(state.session_titles(&page, &CancelToken::new()).is_ok());
    let beyond: Vec<String> = (0..51).map(native).collect();
    assert!(matches!(
        state.session_titles(&beyond, &CancelToken::new()),
        Err(StateError::InvalidTitleRequest)
    ));
    assert!(matches!(
        state.session_titles(&[renamed.clone(), renamed], &CancelToken::new()),
        Err(StateError::InvalidTitleRequest)
    ));
}

/// A cancelled read returns nothing at all, so a stale view is never given
/// titles for rows it no longer shows.
#[test]
fn a_cancelled_read_returns_no_title() {
    let root = tempfile::tempdir().unwrap();
    let (state, ids) = three_sessions(root.path());
    let cancel = CancelToken::new();
    cancel.cancel();
    assert_eq!(
        state.session_titles(&ids, &cancel).unwrap(),
        SessionTitles::default()
    );
}

/// Reading titles stores nothing and changes nothing the list reads: the page
/// is identical before and after, its rows still carry no saved title, and a
/// search for a transient title finds nothing, because search is over the
/// index only.
#[test]
fn titles_leave_the_list_search_and_storage_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let (state, ids) = three_sessions(root.path());
    let db = root.path().join("data/xtrace.db");
    let source = root
        .path()
        .join("home/.claude/projects")
        .join(PROJECT)
        .join(format!("{}.jsonl", ids[0]));
    let before_page = state.sessions_list("", None, None, 7).unwrap();
    let before_source = fs::read(&source).unwrap();
    let before_db = fs::read(&db).unwrap();
    assert_eq!(titles(&state, &ids).titles.len(), 2);
    let after_page = state.sessions_list("", None, None, 7).unwrap();
    // The window moves with the clock; the rows and paging do not.
    assert_eq!(after_page.rows, before_page.rows);
    assert_eq!(after_page.next, before_page.next);
    assert!(after_page.rows.iter().all(|row| row.title.is_none()));
    assert!(
        state
            .sessions_list("Renamed", None, None, 7)
            .unwrap()
            .rows
            .is_empty()
    );
    assert_eq!(fs::read(&source).unwrap(), before_source);
    assert_eq!(fs::read(&db).unwrap(), before_db);
    let store = Store::open(&db).unwrap();
    for id in &ids {
        assert_eq!(store.session(id).unwrap().unwrap().meta.title, None);
    }
}

/// Fixture startup reads no local history, so it has no host title to give.
#[cfg(feature = "fixtures")]
#[test]
fn fixture_mode_reads_no_title() {
    let state = AppState::build(
        StartupOptions {
            fixture: Some("F1".into()),
            ..Default::default()
        },
        || panic!("fixture mode needs no data directory"),
        || panic!("fixture mode needs no native home"),
    )
    .unwrap();
    let rows = state.sessions_list("", None, None, 7).unwrap().rows;
    let ids: Vec<String> = rows.into_iter().map(|row| row.id).take(50).collect();
    assert!(!ids.is_empty());
    assert_eq!(
        state.session_titles(&ids, &CancelToken::new()).unwrap(),
        SessionTitles::default()
    );
}
