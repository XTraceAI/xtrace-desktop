//! Reading a few visible sessions' host titles from their original sources.
//!
//! Every file here is written by the test: short synthetic lines in each
//! host's shape, never a real conversation. The suite proves the rules a list
//! depends on — a Claude rename outranks a later stale generated title, the
//! last valid title of each kind wins, a Codex rollout name outranks the
//! sidecar and the sidecar never names an unverified rollout, a paginated or
//! inherited Codex history is named by its exact-thread sidecar row alone once
//! every indexed header is verified, a prompt never becomes a title, and
//! anything uncertain, oversized, over budget, late or cancelled leaves the
//! identifier in place — and that a read writes nothing.
#![cfg(unix)]
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use xt_fixtures::TempDb;
use xt_ingest::native::{
    ImportRequest, ProducerSource, import_native,
    readers_cli::{CancelToken, read_pin},
    session_source::IndexedSource,
    session_titles::{
        MAX_CODEX_LOCATORS, MAX_TITLE_SESSIONS, TitleLimits, TitleOutcome, TitleSource,
        TitleTarget, Untitled, read_titles, title_sources,
    },
};
use xt_store::{
    Host, SessionSource, Store,
    batch::{MAX_LOCATOR_ROWS, SourceCursor},
};

const OBSERVED_AT: i64 = 1_788_782_400_000;
const SESSION: &str = "00000000-0000-4000-8000-00000000aaaa";
const OTHER: &str = "00000000-0000-4000-8000-00000000bbbb";
const CODEX: &str = "019a0000-0000-7000-8000-00000000c0de";
const PROJECT: &str = "-repo-fixture";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn turn(session: &str, index: usize, text: &str) -> String {
    json!({
        "uuid": format!("11111111-1111-4111-8111-{index:012}"),
        "type": if index.is_multiple_of(2) { "user" } else { "assistant" },
        "sessionId": session, "cwd": "/repo/fixture",
        "timestamp": format!("2026-09-07T12:00:{:02}.000Z", index % 60),
        "message": {"role": if index.is_multiple_of(2) { "user" } else { "assistant" },
                    "content": [{"type": "text", "text": text}]}
    })
    .to_string()
}

fn ai(session: &str, title: &str) -> String {
    json!({"type": "ai-title", "aiTitle": title, "sessionId": session}).to_string()
}

fn rename(session: &str, title: &str) -> String {
    json!({"type": "custom-title", "customTitle": title, "sessionId": session}).to_string()
}

fn lines(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

fn claude_home(root: &Path) -> PathBuf {
    let home = root.join("home");
    fs::create_dir_all(home.join(".claude/projects").join(PROJECT)).unwrap();
    home
}

fn transcript(home: &Path, project: &str, session: &str) -> PathBuf {
    home.join(".claude/projects")
        .join(project)
        .join(format!("{session}.jsonl"))
}

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

/// A locator exactly as the index spells one, with no checkpoint.
fn claude_locator(path: &Path) -> IndexedSource {
    IndexedSource {
        locator: format!("claude:{}", path.display()),
        checkpoint: None,
    }
}

fn codex_locator(path: &Path) -> IndexedSource {
    IndexedSource {
        locator: format!("codex:{}", path.display()),
        checkpoint: None,
    }
}

fn target(host: Host, native: &str, locators: Vec<IndexedSource>) -> TitleTarget {
    TitleTarget {
        host,
        native_session_id: native.to_owned(),
        locators,
    }
}

fn read(home: &Path, targets: &[TitleTarget]) -> Vec<TitleOutcome> {
    read_titles(home, targets, TitleLimits::default(), None).outcomes
}

fn titled(title: &str, source: TitleSource) -> TitleOutcome {
    TitleOutcome::Titled {
        title: title.to_owned(),
        source,
    }
}

/// Index the home exactly as the app does, so locators and checkpoints are the
/// ones a real read is judged against.
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

fn indexed_target(store: &Store, native: &str) -> TitleTarget {
    target(
        Host::Claude,
        native,
        title_sources(store, Host::Claude, native).unwrap(),
    )
}

/// A rename outranks every generated title, whatever was written last, and
/// within each kind the last valid value wins. Malformed values, blank ones,
/// sidechain records and records naming another session are not titles.
#[test]
fn a_claude_rename_outranks_a_later_stale_generated_title() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let path = transcript(&home, PROJECT, SESSION);
    write(
        &path,
        &lines(&[
            turn(SESSION, 0, "please fix the custom-title parser"),
            ai(SESSION, "Generated first"),
            rename(SESSION, "Renamed once"),
            ai(SESSION, "Generated stale"),
            rename(SESSION, "Renamed twice"),
            json!({"type": "custom-title", "customTitle": 7, "sessionId": SESSION}).to_string(),
            json!({"type": "custom-title", "customTitle": "   ", "sessionId": SESSION}).to_string(),
            json!({"type": "custom-title", "customTitle": "Sidechain", "sessionId": SESSION,
                   "isSidechain": true})
            .to_string(),
            rename(OTHER, "Another session's rename"),
            ai(SESSION, "Generated last"),
            turn(SESSION, 1, "done"),
        ]),
    );
    let targets = [target(Host::Claude, SESSION, vec![claude_locator(&path)])];
    assert_eq!(
        read(&home, &targets),
        [titled("Renamed twice", TitleSource::ClaudeRename)]
    );
}

#[test]
fn without_a_rename_the_last_valid_generated_title_names_a_claude_session() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let path = transcript(&home, PROJECT, SESSION);
    write(
        &path,
        &lines(&[
            ai(SESSION, "Early"),
            turn(SESSION, 0, "ask"),
            ai(SESSION, "Later"),
            json!({"type": "ai-title", "aiTitle": null, "sessionId": SESSION}).to_string(),
            "{not json -title\"".to_owned(),
        ]),
    );
    let targets = [target(Host::Claude, SESSION, vec![claude_locator(&path)])];
    assert_eq!(
        read(&home, &targets),
        [titled("Later", TitleSource::ClaudeGenerated)]
    );
}

/// A session with no title record keeps its identifier, however much it said:
/// a prompt is never made into a title.
#[test]
fn a_claude_session_without_a_title_record_has_none() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let path = transcript(&home, PROJECT, SESSION);
    write(
        &path,
        &lines(&[
            turn(SESSION, 0, "Refactor the parser"),
            turn(SESSION, 1, "ok"),
        ]),
    );
    let targets = [target(Host::Claude, SESSION, vec![claude_locator(&path)])];
    assert_eq!(
        read(&home, &targets),
        [TitleOutcome::Untitled(Untitled::NoTitle)]
    );
}

/// A final line without its newline is a write in progress, not a record.
#[test]
fn a_torn_final_title_line_is_not_read() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let path = transcript(&home, PROJECT, SESSION);
    let mut body = lines(&[ai(SESSION, "Complete")]);
    body.push_str(&rename(SESSION, "Torn"));
    write(&path, &body);
    let targets = [target(Host::Claude, SESSION, vec![claude_locator(&path)])];
    assert_eq!(
        read(&home, &targets),
        [titled("Complete", TitleSource::ClaudeGenerated)]
    );
}

/// A complete line longer than the inspected bound is never partly parsed,
/// and it could be the latest rename: an earlier title is not shown in its
/// place, so the source keeps its identifier unless a later valid rename
/// settles it. A torn final line stays ignored, however long.
#[test]
fn an_overlong_complete_line_leaves_the_source_untitled() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let path = transcript(&home, PROJECT, SESSION);
    let limits = TitleLimits {
        max_line_bytes: 1024,
        ..TitleLimits::default()
    };
    let targets = [target(Host::Claude, SESSION, vec![claude_locator(&path)])];
    let earlier = [ai(SESSION, "Short"), rename(SESSION, "Early rename")];
    let read_with = |body: &str| {
        write(&path, body);
        read_titles(&home, &targets, limits, None).outcomes
    };
    // A later rename too long to inspect, then a stale generated title.
    let later_rename = rename(SESSION, &"L".repeat(4096));
    assert_eq!(
        read_with(&lines(&[
            earlier[0].clone(),
            earlier[1].clone(),
            later_rename.clone(),
            ai(SESSION, "Stale after"),
        ])),
        [TitleOutcome::Untitled(Untitled::LineTooLong)]
    );
    // Any overlong complete line: what it holds is not known, so only a
    // later rename names the session.
    assert_eq!(
        read_with(&lines(&[
            earlier[0].clone(),
            turn(SESSION, 0, &"x".repeat(4096)),
        ])),
        [TitleOutcome::Untitled(Untitled::LineTooLong)]
    );
    assert_eq!(
        read_with(&lines(&[
            earlier[0].clone(),
            turn(SESSION, 0, &"x".repeat(4096)),
            earlier[1].clone(),
        ])),
        [titled("Early rename", TitleSource::ClaudeRename)]
    );
    // The same rename straddling the reader's 256 KiB chunks.
    let mut straddling = lines(&earlier);
    let mut index = 0;
    while straddling.len() < 256 * 1024 - 2048 {
        straddling.push_str(&lines(&[turn(SESSION, index, "filler")]));
        index += 1;
    }
    straddling.push_str(&lines(&[later_rename.clone(), ai(SESSION, "Stale after")]));
    assert_eq!(
        read_with(&straddling),
        [TitleOutcome::Untitled(Untitled::LineTooLong)]
    );
    // Torn at the end, it is a write in progress: the earlier rename stands.
    let mut torn = lines(&earlier);
    torn.push_str(&later_rename);
    assert_eq!(
        read_with(&torn),
        [titled("Early rename", TitleSource::ClaudeRename)]
    );
}

const SHORT_LINES: TitleLimits = TitleLimits {
    max_line_bytes: 1024,
    max_file_bytes: xt_ingest::native::session_titles::MAX_TITLE_FILE_BYTES,
    max_batch_bytes: xt_ingest::native::session_titles::MAX_TITLE_BATCH_BYTES,
    deadline: xt_ingest::native::session_titles::TITLE_DEADLINE,
};

/// A synthetic tool result too long for [`SHORT_LINES`] to inspect.
fn opaque(index: usize) -> String {
    turn(SESSION, index, &"x".repeat(4096))
}

/// Each overlong complete line could be a later rename; only a valid rename
/// after it settles it, the last such rename names the session and outranks
/// any later generated title. A generated title settles nothing, and another
/// overlong line unsettles it again.
#[test]
fn a_later_valid_rename_settles_the_overlong_lines_before_it() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let path = transcript(&home, PROJECT, SESSION);
    let targets = [target(Host::Claude, SESSION, vec![claude_locator(&path)])];
    let read_with = |body: &str| {
        write(&path, body);
        read_titles(&home, &targets, SHORT_LINES, None).outcomes
    };
    let untitled = [TitleOutcome::Untitled(Untitled::LineTooLong)];
    let cases: [(&str, Vec<String>, &[TitleOutcome]); 7] = [
        (
            "overlong tool result, then a rename",
            vec![turn(SESSION, 0, "ask"), opaque(1), rename(SESSION, "Kept")],
            &[titled("Kept", TitleSource::ClaudeRename)],
        ),
        (
            "several overlong lines, the last rename wins",
            vec![
                opaque(0),
                rename(SESSION, "First"),
                opaque(1),
                opaque(2),
                rename(SESSION, "Second"),
                rename(SESSION, "Third"),
            ],
            &[titled("Third", TitleSource::ClaudeRename)],
        ),
        (
            "a settled rename outranks a later generated title",
            vec![
                opaque(0),
                rename(SESSION, "Renamed"),
                ai(SESSION, "Generated after"),
            ],
            &[titled("Renamed", TitleSource::ClaudeRename)],
        ),
        (
            "an overlong line after the last rename",
            vec![rename(SESSION, "Before"), opaque(0)],
            &untitled,
        ),
        (
            "another overlong line unsettles it again",
            vec![opaque(0), rename(SESSION, "Settled"), opaque(1)],
            &untitled,
        ),
        (
            "a generated title settles nothing",
            vec![ai(SESSION, "Before"), opaque(0), ai(SESSION, "After")],
            &untitled,
        ),
        (
            "an overlong rename, then a generated title",
            vec![
                rename(SESSION, &"L".repeat(4096)),
                ai(SESSION, "Stale after"),
            ],
            &untitled,
        ),
    ];
    for (label, body, want) in cases {
        assert_eq!(read_with(&lines(&body)), want, "{label}");
    }

    // An overlong line straddling the reader's 256 KiB chunks, settled by a
    // rename in a later chunk.
    let mut straddling = String::new();
    let mut index = 0;
    while straddling.len() < 256 * 1024 - 2048 {
        straddling.push_str(&lines(&[turn(SESSION, index, "filler")]));
        index += 1;
    }
    straddling.push_str(&lines(&[opaque(index), ai(SESSION, "Generated")]));
    assert_eq!(read_with(&straddling), untitled, "straddling, unsettled");
    while straddling.len() < 2 * 256 * 1024 {
        straddling.push_str(&lines(&[turn(SESSION, index, "filler")]));
        index += 1;
    }
    straddling.push_str(&lines(&[rename(SESSION, "Across chunks")]));
    assert_eq!(
        read_with(&straddling),
        [titled("Across chunks", TitleSource::ClaudeRename)],
        "straddling, settled"
    );

    // A torn overlong final line is a write in progress: it neither unsettles
    // nor settles anything.
    let mut torn = lines(&[rename(SESSION, "Earlier")]);
    torn.push_str(&opaque(0));
    assert_eq!(
        read_with(&torn),
        [titled("Earlier", TitleSource::ClaudeRename)]
    );
    let mut torn = lines(&[opaque(0)]);
    torn.push_str(&rename(SESSION, "Torn"));
    assert_eq!(read_with(&torn), untitled, "a torn rename settles nothing");
}

/// Only a complete, valid, top-level rename of this session settles an
/// overlong line: not a malformed, blank, nested, sidechain or other
/// session's record, and not one with a repeated top-level key, however the
/// key is spelled, because which value is meant is not known.
#[test]
fn only_a_valid_distinct_rename_of_this_session_settles_an_overlong_line() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let path = transcript(&home, PROJECT, SESSION);
    let targets = [target(Host::Claude, SESSION, vec![claude_locator(&path)])];
    let read_with = |body: &str| {
        write(&path, body);
        read_titles(&home, &targets, SHORT_LINES, None).outcomes
    };
    let s = SESSION;
    let o = OTHER;
    let candidates = [
        json!({"type": "custom-title", "customTitle": 7, "sessionId": s}).to_string(),
        json!({"type": "custom-title", "customTitle": "  ", "sessionId": s}).to_string(),
        json!({"type": "custom-title", "customTitle": null, "sessionId": s}).to_string(),
        json!({"type": "custom-title", "sessionId": s}).to_string(),
        json!({"type": "user", "message": {"type": "custom-title",
               "customTitle": "Nested", "sessionId": s}})
        .to_string(),
        json!({"type": "custom-title", "customTitle": "Side", "sessionId": s,
               "isSidechain": true})
        .to_string(),
        rename(o, "Other session"),
        json!({"type": "custom-title", "customTitle": "Numeric", "sessionId": 7}).to_string(),
        format!(r#"{{"type":"custom-title","customTitle":"Malformed","sessionId":"{s}""#),
        format!(r#"{{"type":"custom-title","customTitle":"Trailing","sessionId":"{s}"}} x"#),
        format!(r#"{{"type":"user","type":"custom-title","customTitle":"T","sessionId":"{s}"}}"#),
        format!(
            r#"{{"type":"custom-title","customTitle":"A","customTitle":"B","sessionId":"{s}"}}"#
        ),
        format!(
            r#"{{"type":"custom-title","customTitle":"S","sessionId":"{o}","sessionId":"{s}"}}"#
        ),
        format!(
            r#"{{"type":"custom-title","customTitle":"C","sessionId":"{s}","isSidechain":true,"isSidechain":false}}"#
        ),
        format!(
            r#"{{"type":"user","\u0074ype":"custom-title","customTitle":"E","sessionId":"{s}"}}"#
        ),
        format!(
            r#"{{"type":"custom-title","customTitle":"A","custom\u0054itle":"B","sessionId":"{s}"}}"#
        ),
        format!(
            r#"{{"type":"custom-title","customTitle":"U","sessionId":"{s}","uuid":"a","uuid":"b"}}"#
        ),
        format!(
            r#"{{"type":"custom-title","customTitle":"I","sessionId":"{o}","session\u0049d":"{s}"}}"#
        ),
        format!(
            r#"{{"type":"custom-title","customTitle":"K","sessionId":"{s}","isSidechain":true,"is\u0053idechain":false}}"#
        ),
    ];
    for candidate in &candidates {
        assert_eq!(
            read_with(&lines(&[opaque(0), candidate.clone()])),
            [TitleOutcome::Untitled(Untitled::LineTooLong)],
            "{candidate}"
        );
    }
    // Once a valid rename settled it, an ambiguous or invalid rename replaces
    // nothing: the settled name stands until another valid rename, and after
    // a further overlong line it settles nothing either.
    for candidate in &candidates {
        let settled = [opaque(0), rename(SESSION, "Safe"), candidate.clone()];
        assert_eq!(
            read_with(&lines(&settled)),
            [titled("Safe", TitleSource::ClaudeRename)],
            "{candidate}"
        );
        let mut replaced = settled.to_vec();
        replaced.push(rename(SESSION, "Later"));
        assert_eq!(
            read_with(&lines(&replaced)),
            [titled("Later", TitleSource::ClaudeRename)],
            "{candidate}"
        );
        assert_eq!(
            read_with(&lines(&[
                opaque(0),
                rename(SESSION, "Safe"),
                opaque(1),
                candidate.clone(),
            ])),
            [TitleOutcome::Untitled(Untitled::LineTooLong)],
            "{candidate}"
        );
    }
    // A valid rename after all of them still settles the line before it.
    let mut body = vec![opaque(0)];
    body.extend(candidates);
    body.push(rename(SESSION, "Valid"));
    assert_eq!(
        read_with(&lines(&body)),
        [titled("Valid", TitleSource::ClaudeRename)]
    );
    // A rename that names no session, as a rename may, is valid here too.
    assert_eq!(
        read_with(&lines(&[
            opaque(0),
            json!({"type": "custom-title", "customTitle": "Unnamed session"}).to_string(),
        ])),
        [titled("Unnamed session", TitleSource::ClaudeRename)]
    );
}

/// A recovered name still loses to every refusal of the source: a cancel, the
/// deadline, the budget, the file ceiling and a changed generation each leave
/// the identifier; an indexed transcript that only grew is still named.
#[test]
fn a_settled_overlong_line_keeps_every_source_guard() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let path = transcript(&home, PROJECT, SESSION);
    let body = lines(&[
        turn(SESSION, 0, "ask"),
        opaque(1),
        rename(SESSION, "Original"),
    ]);
    write(&path, &body);
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_target(db.store(), SESSION);
    assert!(indexed.locators[0].checkpoint.is_some());
    let targets = [indexed.clone()];
    let batch = read_titles(&home, &targets, SHORT_LINES, None);
    assert_eq!(
        batch.outcomes,
        [titled("Original", TitleSource::ClaudeRename)]
    );
    // The whole file is read, overlong line included, once proven by its
    // recorded tail.
    let len = body.len() as u64;
    assert!(
        batch.bytes_read >= len && batch.bytes_read <= 2 * len,
        "{}",
        batch.bytes_read
    );

    let cancel = CancelToken::new();
    cancel.cancel();
    assert_eq!(
        read_titles(&home, &targets, SHORT_LINES, Some(&cancel)).outcomes,
        [TitleOutcome::Untitled(Untitled::Cancelled)]
    );
    for (limits, why) in [
        (
            TitleLimits {
                deadline: Duration::ZERO,
                ..SHORT_LINES
            },
            Untitled::Deadline,
        ),
        (
            TitleLimits {
                max_batch_bytes: len,
                ..SHORT_LINES
            },
            Untitled::Budget,
        ),
        (
            TitleLimits {
                max_file_bytes: len - 1,
                ..SHORT_LINES
            },
            Untitled::TooLarge,
        ),
    ] {
        assert_eq!(
            read_titles(&home, &targets, limits, None).outcomes,
            [TitleOutcome::Untitled(why)],
            "{why:?}"
        );
    }

    let mut grown = body.clone();
    grown.push_str(&lines(&[opaque(2), rename(SESSION, "Grown")]));
    fs::write(&path, &grown).unwrap();
    assert_eq!(
        read_titles(&home, &targets, SHORT_LINES, None).outcomes,
        [titled("Grown", TitleSource::ClaudeRename)]
    );

    let rewritten = grown.replace("Original", "Imposter");
    assert_eq!(rewritten.len(), grown.len());
    fs::write(&path, &rewritten).unwrap();
    assert_eq!(
        read_titles(&home, &targets, SHORT_LINES, None).outcomes,
        [TitleOutcome::Untitled(Untitled::Replaced)]
    );
}

/// Indexed, then appended: the index's generation still holds inside the
/// file, so the latest title is read from the grown file.
#[test]
fn an_indexed_transcript_that_grew_is_titled_by_its_latest_title() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let path = transcript(&home, PROJECT, SESSION);
    write(
        &path,
        &lines(&[turn(SESSION, 0, "ask"), ai(SESSION, "First")]),
    );
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_target(db.store(), SESSION);
    assert_eq!(indexed.locators.len(), 1);
    assert!(indexed.locators[0].checkpoint.is_some());
    assert_eq!(
        read(&home, std::slice::from_ref(&indexed)),
        [titled("First", TitleSource::ClaudeGenerated)]
    );
    let mut grown = fs::read_to_string(&path).unwrap();
    grown.push_str(&lines(&[rename(SESSION, "Renamed")]));
    fs::write(&path, grown).unwrap();
    assert_eq!(
        read(&home, &[indexed]),
        [titled("Renamed", TitleSource::ClaudeRename)]
    );
}

/// Another file at the indexed name, or the indexed bytes rewritten in place,
/// is not the generation the index measured: no title.
#[test]
fn a_replaced_or_rewritten_transcript_has_no_title() {
    for change in ["replace", "rewrite"] {
        let temp = tempfile::TempDir::new().unwrap();
        let home = claude_home(temp.path());
        let path = transcript(&home, PROJECT, SESSION);
        let body = lines(&[turn(SESSION, 0, "ask"), ai(SESSION, "Original")]);
        write(&path, &body);
        let mut db = TempDb::empty().unwrap();
        index(db.store_mut(), &home);
        let indexed = indexed_target(db.store(), SESSION);
        let changed = body.replace("Original", "Imposter");
        assert_eq!(changed.len(), body.len());
        if change == "replace" {
            let staged = path.with_extension("staged");
            fs::write(&staged, &changed).unwrap();
            fs::rename(&staged, &path).unwrap();
        } else {
            fs::write(&path, &changed).unwrap();
        }
        assert_eq!(
            read(&home, &[indexed]),
            [TitleOutcome::Untitled(Untitled::Replaced)],
            "{change}"
        );
    }
}

/// A transcript that moved to another project is not looked for there: the
/// index recorded one place, and a name match elsewhere is not a guess this
/// read makes.
#[test]
fn a_moved_transcript_is_not_found_by_name_elsewhere() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let path = transcript(&home, PROJECT, SESSION);
    write(
        &path,
        &lines(&[turn(SESSION, 0, "ask"), ai(SESSION, "Moved")]),
    );
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let indexed = indexed_target(db.store(), SESSION);
    assert_eq!(indexed.locators.len(), 1);
    let moved = transcript(&home, "-repo-elsewhere", SESSION);
    fs::create_dir_all(moved.parent().unwrap()).unwrap();
    fs::rename(&path, &moved).unwrap();
    assert_eq!(
        read(&home, &[indexed]),
        [TitleOutcome::Untitled(Untitled::Missing)]
    );
}

/// An alias is never followed — not at the transcript, not at its project —
/// and a locator outside the history root, or naming another session's file,
/// is never read.
#[test]
fn an_alias_an_outside_path_or_another_sessions_file_is_never_read() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let real = temp.path().join("elsewhere");
    let outside = real.join(format!("{SESSION}.jsonl"));
    write(&outside, &lines(&[ai(SESSION, "Outside")]));
    // The transcript itself is an alias.
    let aliased = transcript(&home, PROJECT, SESSION);
    std::os::unix::fs::symlink(&outside, &aliased).unwrap();
    // The project directory is an alias.
    let aliased_project = home.join(".claude/projects/-repo-alias");
    std::os::unix::fs::symlink(&real, &aliased_project).unwrap();
    let through_project = aliased_project.join(format!("{SESSION}.jsonl"));
    // Another session's own transcript, recorded under this session.
    let other = transcript(&home, PROJECT, OTHER);
    write(&other, &lines(&[ai(OTHER, "Other")]));
    for locator in [&aliased, &through_project, &outside, &other] {
        let targets = [target(Host::Claude, SESSION, vec![claude_locator(locator)])];
        assert_eq!(
            read(&home, &targets),
            [TitleOutcome::Untitled(Untitled::Outside)],
            "{}",
            locator.display()
        );
    }
}

#[test]
fn two_distinct_files_claiming_one_session_are_ambiguous() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let first = transcript(&home, PROJECT, SESSION);
    let second = transcript(&home, "-repo-twin", SESSION);
    write(&first, &lines(&[ai(SESSION, "One")]));
    write(&second, &lines(&[ai(SESSION, "Two")]));
    let targets = [target(
        Host::Claude,
        SESSION,
        vec![claude_locator(&first), claude_locator(&second)],
    )];
    assert_eq!(
        read(&home, &targets),
        [TitleOutcome::Untitled(Untitled::Ambiguous)]
    );
    // Two spellings of one file are one file.
    let targets = [target(
        Host::Claude,
        SESSION,
        vec![claude_locator(&first), claude_locator(&first)],
    )];
    assert_eq!(
        read(&home, &targets),
        [titled("One", TitleSource::ClaudeGenerated)]
    );
}

/// A cancel returns no title for any row, and ceilings, the batch budget and
/// the deadline leave the rows they reach with their identifiers.
#[test]
fn cancelled_oversized_over_budget_and_late_reads_have_no_title() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let first = transcript(&home, PROJECT, SESSION);
    let second = transcript(&home, PROJECT, OTHER);
    write(&first, &lines(&[ai(SESSION, "First")]));
    write(&second, &lines(&[ai(OTHER, "Second")]));
    let targets = [
        target(Host::Claude, SESSION, vec![claude_locator(&first)]),
        target(Host::Claude, OTHER, vec![claude_locator(&second)]),
    ];
    let cancel = CancelToken::new();
    cancel.cancel();
    let batch = read_titles(&home, &targets, TitleLimits::default(), Some(&cancel));
    assert_eq!(
        batch.outcomes,
        [
            TitleOutcome::Untitled(Untitled::Cancelled),
            TitleOutcome::Untitled(Untitled::Cancelled)
        ]
    );
    assert_eq!(batch.bytes_read, 0);

    let small = fs::metadata(&first).unwrap().len();
    let too_large = TitleLimits {
        max_file_bytes: small - 1,
        ..TitleLimits::default()
    };
    assert_eq!(
        read_titles(&home, &targets[..1], too_large, None).outcomes,
        [TitleOutcome::Untitled(Untitled::TooLarge)]
    );

    let one_file = TitleLimits {
        max_batch_bytes: small,
        ..TitleLimits::default()
    };
    assert_eq!(
        read_titles(&home, &targets, one_file, None).outcomes,
        [
            titled("First", TitleSource::ClaudeGenerated),
            TitleOutcome::Untitled(Untitled::Budget)
        ]
    );

    let late = TitleLimits {
        deadline: Duration::ZERO,
        ..TitleLimits::default()
    };
    let batch = read_titles(&home, &targets, late, None);
    assert_eq!(
        batch.outcomes,
        [
            TitleOutcome::Untitled(Untitled::Deadline),
            TitleOutcome::Untitled(Untitled::Deadline)
        ]
    );
    assert_eq!(batch.bytes_read, 0);
}

/// Codex's own rollout name wins; the last valid one when it was renamed.
/// Without one the sidecar may name a verified rollout; with neither, the
/// session's prompt is not made into a title.
#[test]
fn codex_titles_come_from_the_rollout_then_the_sidecar_and_never_the_prompt() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let named = rollout_path(&home, CODEX);
    write(
        &named,
        &rollout(
            CODEX,
            &[
                thread_name(CODEX, "Codex first"),
                thread_name(CODEX, "Codex renamed"),
                json!({"type": "event_msg", "payload": {"type": "thread_name_updated",
                        "thread_id": CODEX, "thread_name": ""}})
                .to_string(),
            ],
        ),
    );
    let unnamed_id = "019a0000-0000-7000-8000-0000000000aa";
    let unnamed = rollout_path(&home, unnamed_id);
    write(&unnamed, &rollout(unnamed_id, &[]));
    let bare_id = "019a0000-0000-7000-8000-0000000000bb";
    let bare = rollout_path(&home, bare_id);
    write(&bare, &rollout(bare_id, &[]));
    write(
        &home.join(".codex/session_index.jsonl"),
        &lines(&[
            json!({"id": CODEX, "thread_name": "Sidecar loses to the rollout"}).to_string(),
            json!({"id": unnamed_id, "thread_name": "Sidecar old"}).to_string(),
            json!({"id": unnamed_id, "thread_name": "Sidecar name"}).to_string(),
            json!({"id": unnamed_id, "thread_name": 5}).to_string(),
        ]),
    );
    let targets = [
        target(Host::Codex, CODEX, vec![codex_locator(&named)]),
        target(Host::Codex, unnamed_id, vec![codex_locator(&unnamed)]),
        target(Host::Codex, bare_id, vec![codex_locator(&bare)]),
    ];
    assert_eq!(
        read(&home, &targets),
        [
            titled("Codex renamed", TitleSource::CodexRollout),
            titled("Sidecar name", TitleSource::CodexSidecar),
            TitleOutcome::Untitled(Untitled::NoTitle),
        ]
    );
}

/// The sidecar names only a session whose own rollout was verified here: not
/// one whose rollout is gone, names another session, or declares a paginated
/// history anywhere but its opening line.
#[test]
fn the_codex_sidecar_never_names_an_unverified_rollout() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let gone = rollout_path(&home, CODEX);
    let foreign_id = "019a0000-0000-7000-8000-0000000000aa";
    let foreign = rollout_path(&home, foreign_id);
    write(&foreign, &rollout(OTHER, &[thread_name(OTHER, "Foreign")]));
    let late_id = "019a0000-0000-7000-8000-0000000000bb";
    let late = rollout_path(&home, late_id);
    write(
        &late,
        &lines(&[
            json!({"type": "turn_context", "payload": {}}).to_string(),
            paged_header(late_id, None),
            thread_name(late_id, "Late"),
        ]),
    );
    write(
        &home.join(".codex/session_index.jsonl"),
        &lines(&[
            json!({"id": CODEX, "thread_name": "Borrowed"}).to_string(),
            json!({"id": foreign_id, "thread_name": "Borrowed"}).to_string(),
            json!({"id": late_id, "thread_name": "Borrowed"}).to_string(),
        ]),
    );
    let targets = [
        target(Host::Codex, CODEX, vec![codex_locator(&gone)]),
        target(Host::Codex, foreign_id, vec![codex_locator(&foreign)]),
        target(Host::Codex, late_id, vec![codex_locator(&late)]),
    ];
    assert_eq!(
        read(&home, &targets),
        [
            TitleOutcome::Untitled(Untitled::Missing),
            TitleOutcome::Untitled(Untitled::IdentityMismatch),
            TitleOutcome::Untitled(Untitled::UnsupportedHistory),
        ]
    );
}

/// A sidecar that is an alias is not read; its torn last row is not a row.
#[test]
fn an_aliased_sidecar_or_its_torn_row_names_nothing() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let path = rollout_path(&home, CODEX);
    write(&path, &rollout(CODEX, &[]));
    let sidecar = home.join(".codex/session_index.jsonl");
    write(
        &sidecar,
        &json!({"id": CODEX, "thread_name": "Torn"}).to_string(),
    );
    let targets = [target(Host::Codex, CODEX, vec![codex_locator(&path)])];
    assert_eq!(
        read(&home, &targets),
        [TitleOutcome::Untitled(Untitled::NoTitle)]
    );
    let real = temp.path().join("index.jsonl");
    write(
        &real,
        &lines(&[json!({"id": CODEX, "thread_name": "Aliased"}).to_string()]),
    );
    fs::remove_file(&sidecar).unwrap();
    std::os::unix::fs::symlink(&real, &sidecar).unwrap();
    assert_eq!(
        read(&home, &targets),
        [TitleOutcome::Untitled(Untitled::NoTitle)]
    );
}

/// A rollout reached through an aliased directory below the Codex root is not
/// read, nor one outside it.
#[test]
fn a_codex_rollout_outside_its_root_or_behind_an_alias_is_never_read() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let real = temp.path().join("real-day");
    let file_name = format!("rollout-2026-09-07T12-00-00-{CODEX}.jsonl");
    write(
        &real.join(&file_name),
        &rollout(CODEX, &[thread_name(CODEX, "Hidden")]),
    );
    let month = home.join(".codex/sessions/2026/09");
    fs::create_dir_all(&month).unwrap();
    std::os::unix::fs::symlink(&real, month.join("07")).unwrap();
    for path in [month.join("07").join(&file_name), real.join(&file_name)] {
        let targets = [target(Host::Codex, CODEX, vec![codex_locator(&path)])];
        assert_eq!(
            read(&home, &targets),
            [TitleOutcome::Untitled(Untitled::Outside)],
            "{}",
            path.display()
        );
    }
}

/// Cursor has no host title here and nothing is read; rows keep their order
/// and each outcome belongs to the row at its position.
#[test]
fn outcomes_follow_their_targets_and_cursor_reads_nothing() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let first = transcript(&home, PROJECT, SESSION);
    write(&first, &lines(&[ai(SESSION, "Claude row")]));
    let rollout_file = rollout_path(&home, CODEX);
    write(
        &rollout_file,
        &rollout(CODEX, &[thread_name(CODEX, "Codex row")]),
    );
    let cursor = temp.path().join("cursor.jsonl");
    write(&cursor, &lines(&[ai("cursor-x", "Not read")]));
    let targets = [
        target(Host::Cursor, "cursor-x", vec![claude_locator(&cursor)]),
        target(Host::Codex, CODEX, vec![codex_locator(&rollout_file)]),
        target(Host::Claude, OTHER, Vec::new()),
        target(Host::Claude, SESSION, vec![claude_locator(&first)]),
        target(Host::Claude, "../escape", vec![claude_locator(&first)]),
    ];
    let batch = read_titles(&home, &targets, TitleLimits::default(), None);
    assert_eq!(
        batch.outcomes,
        [
            TitleOutcome::Untitled(Untitled::UnsupportedHost),
            titled("Codex row", TitleSource::CodexRollout),
            TitleOutcome::Untitled(Untitled::NotIndexed),
            titled("Claude row", TitleSource::ClaudeGenerated),
            TitleOutcome::Untitled(Untitled::InvalidIdentifier),
        ]
    );
    // The rollout's opening line is probed first — this small rollout is one
    // header chunk — then it is streamed whole.
    let rollout_len = fs::metadata(&rollout_file).unwrap().len();
    let expected = 2 * rollout_len + fs::metadata(&first).unwrap().len();
    assert_eq!(
        batch.bytes_read, expected,
        "only the two titled sources were read"
    );
}

/// The locators a title read may use: a Claude session's own transcript,
/// never a subagent file; a Codex rollout recorded under exactly this native
/// identity; nothing for Cursor or an identifier that could name a path.
#[test]
fn title_sources_name_only_the_sessions_own_recorded_sources() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let path = transcript(&home, PROJECT, SESSION);
    write(&path, &lines(&[turn(SESSION, 0, "ask")]));
    let subagent = home
        .join(".claude/projects")
        .join(PROJECT)
        .join(SESSION)
        .join("subagents/agent-1.jsonl");
    write(
        &subagent,
        &lines(&[json!({
            "uuid": "22222222-2222-4222-8222-000000000000", "type": "user",
            "sessionId": SESSION, "isSidechain": true, "timestamp": "2026-09-07T12:00:05.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "sub"}]}
        })
        .to_string()]),
    );
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let claude = title_sources(db.store(), Host::Claude, SESSION).unwrap();
    assert_eq!(claude.len(), 1);
    assert_eq!(claude[0].locator, format!("claude:{}", path.display()));

    let rollout_file = rollout_path(&home, CODEX);
    let near_miss = rollout_path(&home, "019a0000-0000-7000-8000-00000000c0dex");
    for path in [&rollout_file, &near_miss] {
        db.store_mut()
            .record_native_source_locator(
                &SourceCursor {
                    source: SessionSource::ReadersCli,
                    cursor_key: format!("codex:{}", path.display()),
                    position: 0,
                    updated_at: OBSERVED_AT,
                },
                true,
            )
            .unwrap();
    }
    let codex = title_sources(db.store(), Host::Codex, CODEX).unwrap();
    assert_eq!(
        codex
            .iter()
            .map(|source| source.locator.clone())
            .collect::<Vec<_>>(),
        [format!("codex:{}", rollout_file.display())]
    );
    assert!(
        title_sources(db.store(), Host::Cursor, SESSION)
            .unwrap()
            .is_empty()
    );
    assert!(
        title_sources(db.store(), Host::Claude, "../x")
            .unwrap()
            .is_empty()
    );
}

/// Reading titles writes nothing: the sources are byte-for-byte unchanged and
/// the index holds exactly what it held before.
#[test]
fn a_title_read_changes_no_source_and_no_stored_row() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let path = transcript(&home, PROJECT, SESSION);
    write(
        &path,
        &lines(&[turn(SESSION, 0, "ask"), rename(SESSION, "Kept in memory")]),
    );
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let before_source = fs::read(&path).unwrap();
    let before_db = fs::read(db.path()).unwrap();
    let before_listing = listing(&home);
    let targets = [indexed_target(db.store(), SESSION)];
    assert_eq!(
        read(&home, &targets),
        [titled("Kept in memory", TitleSource::ClaudeRename)]
    );
    assert_eq!(fs::read(&path).unwrap(), before_source);
    assert_eq!(fs::read(db.path()).unwrap(), before_db);
    assert_eq!(listing(&home), before_listing);
    let stored = db.store().session(SESSION).unwrap().unwrap();
    assert_eq!(stored.meta.title, None);
}

/// A full page of sources, each with its title near the end of a sizeable
/// transcript, is read within one batch's bounds: every row is titled, every
/// byte is accounted for, and the batch stays well inside its deadline.
#[test]
fn a_full_page_of_transcripts_is_titled_within_the_batch_bounds() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let filler: Vec<String> = (0..2000)
        .map(|index| turn("s", index, "x".repeat(200).as_str()))
        .collect();
    let mut targets = Vec::new();
    let mut total = 0;
    for index in 0..MAX_TITLE_SESSIONS {
        let native = format!("00000000-0000-4000-8000-{index:012}");
        let path = transcript(&home, PROJECT, &native);
        let mut body = lines(&filler).replace("\"s\"", &format!("\"{native}\""));
        body.push_str(&lines(&[ai(&native, &format!("Title {index}"))]));
        write(&path, &body);
        total += body.len() as u64;
        targets.push(target(Host::Claude, &native, vec![claude_locator(&path)]));
    }
    let started = Instant::now();
    let batch = read_titles(&home, &targets, TitleLimits::default(), None);
    let elapsed = started.elapsed();
    for (index, outcome) in batch.outcomes.iter().enumerate() {
        assert_eq!(outcome.title(), Some(format!("Title {index}").as_str()));
    }
    assert_eq!(batch.bytes_read, total);
    assert!(
        elapsed < Duration::from_secs(5),
        "{elapsed:?} for {total} bytes"
    );
    eprintln!(
        "titled {} sessions, {total} bytes, in {elapsed:?}",
        MAX_TITLE_SESSIONS
    );
}

fn listing(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if fs::symlink_metadata(&path).unwrap().is_dir() {
                pending.push(path.clone());
            }
            found.push(path);
        }
    }
    found.sort();
    found
}

fn rollout_path(home: &Path, native: &str) -> PathBuf {
    home.join(".codex/sessions/2026/09/07")
        .join(format!("rollout-2026-09-07T12-00-00-{native}.jsonl"))
}

fn thread_name(native: &str, name: &str) -> String {
    json!({"timestamp": "2026-09-07T12:00:09.000Z", "type": "event_msg",
           "payload": {"type": "thread_name_updated", "thread_id": native, "thread_name": name}})
    .to_string()
}

/// A rollout whose header names `native`, with a user prompt that must never
/// become its title, then `extra`.
fn rollout(native: &str, extra: &[String]) -> String {
    let mut all = vec![
        json!({"timestamp": "2026-09-07T12:00:00.000Z", "type": "session_meta",
               "payload": {"id": native, "originator": "codex_exec", "cwd": "/repo/fixture",
                           "instructions": "long instructions ".repeat(64)}})
        .to_string(),
        json!({"timestamp": "2026-09-07T12:00:01.000Z", "type": "response_item",
               "payload": {"type": "message", "role": "user",
                           "content": [{"type": "input_text", "text": "Prompt is not a title"}]}})
        .to_string(),
    ];
    all.extend(extra.iter().cloned());
    lines(&all)
}

/// Latency and bytes read for a realistic page, measured by hand in release
/// mode (`cargo test --release -p xt-ingest --test session_titles -- --ignored
/// --nocapture`): fifty indexed transcripts of 1–8 MiB with titles
/// interleaved through them, then the dozen rows a dense Dashboard lane card
/// shows. Every read proves each transcript's checkpoint before streaming it,
/// as the app does. The first read follows indexing, so the files are in the
/// page cache; a cold-cache figure needs a machine where the cache can be
/// dropped.
#[test]
#[ignore = "benchmark; run by hand"]
fn measure_realistic_title_batches() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = claude_home(temp.path());
    let mut natives = Vec::new();
    let mut total = 0_u64;
    for index in 0..MAX_TITLE_SESSIONS {
        let native = format!("00000000-0000-4000-8000-{index:012}");
        let mebibytes = 1 + index % 8;
        let text = "x".repeat(2000);
        let mut body = String::new();
        let mut turn_index = 0;
        while body.len() < mebibytes * 1024 * 1024 {
            // Each session's records have their own identities, as real
            // ones do; a shared UUID would be indexed as a copy.
            let line = turn(&native, turn_index, &text).replace(
                "11111111-1111-4111-8111-",
                &format!("{index:08x}-1111-4111-8111-"),
            );
            body.push_str(&line);
            body.push('\n');
            if turn_index % 40 == 0 {
                body.push_str(&ai(&native, &format!("Generated {turn_index}")));
                body.push('\n');
            }
            if turn_index % 97 == 0 {
                body.push_str(&rename(&native, &format!("Renamed {turn_index}")));
                body.push('\n');
            }
            turn_index += 1;
        }
        total += body.len() as u64;
        write(&transcript(&home, PROJECT, &native), &body);
        natives.push(native);
    }
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &home);
    let targets: Vec<TitleTarget> = natives
        .iter()
        .map(|native| indexed_target(db.store(), native))
        .collect();
    for (label, batch) in [
        ("page of 50", &targets[..]),
        ("dense lanes, 12", &targets[..12]),
    ] {
        for run in ["first", "second"] {
            let started = Instant::now();
            let read = read_titles(&home, batch, TitleLimits::default(), None);
            let elapsed = started.elapsed();
            let titled = read.outcomes.iter().filter(|o| o.title().is_some()).count();
            eprintln!(
                "{label} ({run}): {titled}/{} titled, {} MiB read, {elapsed:?}",
                batch.len(),
                read.bytes_read / (1024 * 1024)
            );
        }
    }
    eprintln!("page sources total {} MiB", total / (1024 * 1024));
}

// Paginated and inherited Codex histories. A thread's history can span a root
// rollout and continuations named `<thread>_<rollout>`; each opens with a
// `session_meta` whose `payload.id` is the thread and whose `history_base`
// names the rollout it continues from. Only those opening lines are read, and
// only the thread's exact-ID sidecar row may name it.

const ROLLOUT_A: &str = "019a0000-0000-7000-8000-0000000a0001";
const ROLLOUT_B: &str = "019a0000-0000-7000-8000-0000000a0002";
const PARENT: &str = "019a0000-0000-7000-8000-00000000fa00";

/// A continuation of `thread` with its own immutable rollout identity.
fn continuation_path(home: &Path, day: &str, thread: &str, rollout: &str) -> PathBuf {
    home.join(".codex/sessions/2026/09").join(day).join(format!(
        "rollout-2026-09-{day}T12-00-00-{thread}_{rollout}.jsonl"
    ))
}

/// A paginated `session_meta`, continuing from `base` when it has one.
fn paged_header(thread: &str, base: Option<&str>) -> String {
    let history_base = base.map_or(
        serde_json::Value::Null,
        |rollout| json!({"thread_id": rollout, "end_byte_offset": 10, "end_ordinal_exclusive": 1}),
    );
    json!({"timestamp": "2026-09-07T12:00:00.000Z", "type": "session_meta",
           "ordinal": if base.is_some() { 1 } else { 0 },
           "payload": {"id": thread, "session_id": thread, "history_mode": "paginated",
                       "history_base": history_base, "cwd": "/repo/fixture",
                       "instructions": "long instructions ".repeat(64)}})
    .to_string()
}

/// A paginated segment: its header, a prompt that must never become a title,
/// then `extra`.
fn paged(thread: &str, base: Option<&str>, extra: &[String]) -> String {
    let mut all = vec![
        paged_header(thread, base),
        json!({"type": "response_item",
               "payload": {"type": "message", "role": "user",
                           "content": [{"type": "input_text", "text": "Prompt is not a title"}]}})
        .to_string(),
    ];
    all.extend(extra.iter().cloned());
    lines(&all)
}

fn sidecar(home: &Path, rows: &[(&str, &str)]) -> PathBuf {
    let path = home.join(".codex/session_index.jsonl");
    write(
        &path,
        &lines(
            &rows
                .iter()
                .map(|(id, name)| {
                    json!({"id": id, "thread_name": name, "updated_at": "2026-09-07T12:00:00Z"})
                        .to_string()
                })
                .collect::<Vec<_>>(),
        ),
    );
    path
}

fn codex(native: &str, paths: &[&PathBuf]) -> TitleTarget {
    target(
        Host::Codex,
        native,
        paths.iter().map(|path| codex_locator(path)).collect(),
    )
}

/// The local sample's shape: one paginated root with no rename in it and a
/// matching sidecar row. The last valid exact-ID row names it; only its
/// opening line is read, however large its body — even beyond the per-source
/// ceiling a flat rollout is streamed within.
#[test]
fn a_paginated_root_is_named_by_its_last_exact_sidecar_row() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let root = rollout_path(&home, CODEX);
    let body: Vec<String> = (0..400)
        .map(|index| turn(CODEX, index, &"prompt body ".repeat(400)))
        .collect();
    write(&root, &paged(CODEX, None, &body));
    let header_len = paged_header(CODEX, None).len() as u64 + 1;
    let file_len = fs::metadata(&root).unwrap().len();
    assert!(file_len > 1024 * 1024);
    let index = sidecar(
        &home,
        &[
            (CODEX, "Sidecar older"),
            (OTHER, "Another thread"),
            (CODEX, "Sidecar newest"),
        ],
    );
    let index_len = fs::metadata(&index).unwrap().len();
    let limits = TitleLimits {
        max_file_bytes: 64 * 1024,
        ..TitleLimits::default()
    };
    let batch = read_titles(&home, &[codex(CODEX, &[&root])], limits, None);
    assert_eq!(
        batch.outcomes,
        [titled("Sidecar newest", TitleSource::CodexSidecar)]
    );
    // The header and at most one small chunk past it, then the sidecar.
    assert!(
        batch.bytes_read <= header_len + 16 * 1024 + index_len,
        "{} bytes read of a {file_len}-byte rollout",
        batch.bytes_read
    );

    // With no sidecar there is no title: the rollout body is not a fallback.
    fs::remove_file(&index).unwrap();
    assert_eq!(
        read(&home, &[codex(CODEX, &[&root])]),
        [TitleOutcome::Untitled(Untitled::NoTitle)]
    );
}

/// Only a continuation was recorded: its `<thread>_<rollout>` name and its own
/// header prove the thread, and the thread's sidecar row names it.
#[test]
fn a_continuation_only_locator_is_proven_by_its_own_header() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let part = continuation_path(&home, "08", CODEX, ROLLOUT_A);
    write(&part, &paged(CODEX, Some(CODEX), &[]));
    sidecar(&home, &[(CODEX, "Thread name")]);
    assert_eq!(
        read(&home, &[codex(CODEX, &[&part])]),
        [titled("Thread name", TitleSource::CodexSidecar)]
    );
}

/// A root and two continuations sharing the thread with distinct rollout
/// identities are one history. A rename inside the root, superseded by a later
/// continuation's, is never the thread's name: only the sidecar names it, and
/// without a sidecar row the thread keeps its identifier.
#[test]
fn a_segment_rename_never_names_a_paginated_thread() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let root = rollout_path(&home, CODEX);
    let first = continuation_path(&home, "08", CODEX, ROLLOUT_A);
    let second = continuation_path(&home, "09", CODEX, ROLLOUT_B);
    write(
        &root,
        &paged(CODEX, None, &[thread_name(CODEX, "Root stale")]),
    );
    write(&first, &paged(CODEX, Some(CODEX), &[]));
    write(
        &second,
        &paged(
            CODEX,
            Some(ROLLOUT_A),
            &[thread_name(CODEX, "Continuation newer")],
        ),
    );
    let index = sidecar(&home, &[(CODEX, "Sidecar name")]);
    let whole = codex(CODEX, &[&root, &first, &second]);
    // The usual record: the index names a paginated history by its root.
    let root_only = codex(CODEX, &[&root]);
    assert_eq!(
        read(&home, &[whole.clone(), root_only.clone()]),
        [
            titled("Sidecar name", TitleSource::CodexSidecar),
            titled("Sidecar name", TitleSource::CodexSidecar),
        ]
    );
    fs::remove_file(&index).unwrap();
    assert_eq!(
        read(&home, &[whole, root_only]),
        [
            TitleOutcome::Untitled(Untitled::NoTitle),
            TitleOutcome::Untitled(Untitled::NoTitle),
        ]
    );
}

/// A fork's `history_base` names its parent's rollout, while its own
/// `payload.id` names the child thread. The child is never given the parent's
/// sidecar name, and a sidecar row keyed by a rollout identity names nothing.
#[test]
fn a_fork_never_borrows_its_parents_sidecar_name() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let parent = rollout_path(&home, PARENT);
    write(&parent, &paged(PARENT, None, &[]));
    let child = rollout_path(&home, CODEX);
    write(
        &child,
        &paged(CODEX, Some(PARENT), &[thread_name(CODEX, "Child segment")]),
    );
    let part = continuation_path(&home, "08", PARENT, ROLLOUT_A);
    write(&part, &paged(PARENT, Some(PARENT), &[]));
    let targets = [codex(PARENT, &[&parent, &part]), codex(CODEX, &[&child])];
    sidecar(
        &home,
        &[(PARENT, "Parent name"), (ROLLOUT_A, "Keyed by a rollout")],
    );
    assert_eq!(
        read(&home, &targets),
        [
            titled("Parent name", TitleSource::CodexSidecar),
            TitleOutcome::Untitled(Untitled::NoTitle),
        ]
    );
    sidecar(&home, &[(PARENT, "Parent name"), (CODEX, "Child name")]);
    assert_eq!(
        read(&home, &targets),
        [
            titled("Parent name", TitleSource::CodexSidecar),
            titled("Child name", TitleSource::CodexSidecar),
        ]
    );
}

/// A spawned thread's header names its immediate parent in its typed
/// `thread_spawn` (and, in current headers, in `payload.parent_thread_id`),
/// and the root of its spawn tree in `payload.session_id`: the parent itself
/// at the first level, another thread deeper. Such a header is the thread's
/// own whatever root it shares; an ordinary header keeps a `session_id`
/// naming the thread itself. Anything else — a missing or child-valued root
/// in a spawn, an explicit parent that disagrees with the typed one, a
/// malformed typed parent, a guardian, role or path lookalike, a
/// `history_base`, or another thread's `payload.id` — leaves the thread
/// untitled. The parent's sidecar row never names the child, and a flat
/// header is read as before.
#[test]
fn a_spawned_paginated_thread_is_proven_only_by_its_typed_parent() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let n = |index: u32| format!("019a0000-0000-7000-8000-{index:012}");
    let spawn = |parent: &str| {
        json!({"subagent": {"thread_spawn": {
            "parent_thread_id": parent, "depth": 1,
            "agent_path": "/root/synthetic_task", "agent_nickname": null, "agent_role": null}}})
    };
    // A paginated root header for `thread` with this `source` and
    // `session_id` (none when `None`), and an optional `history_base`.
    let header = |thread: &str,
                  source: serde_json::Value,
                  session: Option<serde_json::Value>,
                  base: Option<&str>| {
        let mut value = json!({"timestamp": "2026-09-07T12:00:00.000Z", "type": "session_meta",
               "payload": {"id": thread, "history_mode": "paginated",
                           "history_base": base.map_or(serde_json::Value::Null,
                               |rollout| json!({"thread_id": rollout, "end_byte_offset": 10})),
                           "source": source, "cwd": "/repo/fixture"}});
        if let Some(session) = session {
            value["payload"]["session_id"] = session;
        }
        value.to_string()
    };
    let paged_body = |first: String| {
        lines(&[
            first,
            json!({"type": "response_item",
                   "payload": {"type": "message", "role": "user",
                               "content": [{"type": "input_text", "text": "Prompt is not a title"}]}})
            .to_string(),
        ])
    };
    let explicit = |line: String, parent: &str| {
        let mut value: serde_json::Value = serde_json::from_str(&line).unwrap();
        value["payload"]["parent_thread_id"] = json!(parent);
        value.to_string()
    };
    let titled_as = |native: &str| titled(&format!("Name {native}"), TitleSource::CodexSidecar);
    let untitled = || TitleOutcome::Untitled(Untitled::IdentityMismatch);
    let mut flat_spawn: serde_json::Value =
        serde_json::from_str(rollout(&n(72), &[]).lines().next().unwrap()).unwrap();
    flat_spawn["payload"]["source"] = spawn(PARENT);
    flat_spawn["payload"]["session_id"] = json!(OTHER);
    let cases: Vec<(String, String, TitleOutcome)> = vec![
        // The observed spawned shape.
        (
            n(61),
            paged_body(header(&n(61), spawn(PARENT), Some(json!(PARENT)), None)),
            titled_as(&n(61)),
        ),
        // An ordinary paginated thread, with its own or no session_id.
        (
            n(62),
            paged_body(header(&n(62), json!("cli"), Some(json!(n(62))), None)),
            titled_as(&n(62)),
        ),
        (
            n(63),
            paged_body(header(&n(63), json!("cli"), None, None)),
            titled_as(&n(63)),
        ),
        // A nested spawn, whose root is another thread, with or without the
        // explicit immediate parent.
        (
            n(74),
            paged_body(explicit(
                header(&n(74), spawn(PARENT), Some(json!(OTHER)), None),
                PARENT,
            )),
            titled_as(&n(74)),
        ),
        // An explicit parent that disagrees with the typed one.
        (
            n(75),
            paged_body(explicit(
                header(&n(75), spawn(PARENT), Some(json!(PARENT)), None),
                OTHER,
            )),
            untitled(),
        ),
        // A spawn whose session_id is missing or the child.
        (
            n(64),
            paged_body(header(&n(64), spawn(PARENT), None, None)),
            untitled(),
        ),
        (
            n(65),
            paged_body(header(&n(65), spawn(PARENT), Some(json!(n(65))), None)),
            untitled(),
        ),
        // A root that is not the typed parent is never compared with it.
        (
            n(66),
            paged_body(header(&n(66), spawn(PARENT), Some(json!(OTHER)), None)),
            titled_as(&n(66)),
        ),
        (
            n(67),
            paged_body(header(&n(67), spawn(OTHER), Some(json!(PARENT)), None)),
            titled_as(&n(67)),
        ),
        // A malformed typed parent.
        (
            n(68),
            paged_body(header(
                &n(68),
                spawn("019a0000"),
                Some(json!("019a0000")),
                None,
            )),
            untitled(),
        ),
        // A parent-valued session_id beside a guardian, a role or path, or a
        // history continued from the parent's rollout.
        (
            n(69),
            paged_body(header(
                &n(69),
                json!({"subagent": {"other": "guardian"}}),
                Some(json!(PARENT)),
                None,
            )),
            untitled(),
        ),
        (
            n(70),
            paged_body(header(
                &n(70),
                json!({"agent_path": "/root/task", "agent_role": "reviewer", "parent_thread_id": PARENT}),
                Some(json!(PARENT)),
                None,
            )),
            untitled(),
        ),
        (
            n(71),
            paged_body(header(
                &n(71),
                json!("cli"),
                Some(json!(PARENT)),
                Some(PARENT),
            )),
            untitled(),
        ),
        // A flat header's session_id is not consulted, as before.
        (n(72), lines(&[flat_spawn.to_string()]), titled_as(&n(72))),
        // Another thread's spawned header under this thread's name.
        (
            n(73),
            paged_body(header(OTHER, spawn(PARENT), Some(json!(PARENT)), None)),
            untitled(),
        ),
    ];
    let mut rows: Vec<(String, String)> = cases
        .iter()
        .map(|(native, _, _)| (native.clone(), format!("Name {native}")))
        .collect();
    rows.push((PARENT.to_owned(), "Parent name".to_owned()));
    let rows: Vec<(&str, &str)> = rows
        .iter()
        .map(|(id, name)| (id.as_str(), name.as_str()))
        .collect();
    let index = sidecar(&home, &rows);
    let mut targets = Vec::new();
    for (native, body, _) in &cases {
        let path = rollout_path(&home, native);
        write(&path, body);
        targets.push(codex(native, &[&path]));
    }
    let outcomes = read(&home, &targets);
    for ((native, _, want), got) in cases.iter().zip(&outcomes) {
        assert_eq!(got, want, "{native}");
    }

    // With no row of its own, a spawned child is never named by its parent's
    // row: it keeps its identifier.
    fs::remove_file(&index).unwrap();
    sidecar(&home, &[(PARENT, "Parent name")]);
    assert_eq!(
        read(&home, &targets[..1]),
        [TitleOutcome::Untitled(Untitled::NoTitle)]
    );
}

/// Headers that disagree with their names or with each other leave the thread
/// untitled, though the sidecar names it.
#[test]
fn foreign_or_conflicting_paginated_headers_leave_the_identifier() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    sidecar(&home, &[(CODEX, "Not shown")]);
    let root = rollout_path(&home, CODEX);
    write(&root, &paged(CODEX, None, &[]));
    let case = |name: &str, body: &str| {
        let path = continuation_path(&home, "08", CODEX, ROLLOUT_A).with_file_name(name);
        write(&path, body);
        path
    };
    let continuation =
        |rollout: &str| format!("rollout-2026-09-08T12-00-00-{CODEX}_{rollout}.jsonl");
    // Another thread's header under this thread's continuation name.
    let foreign = case(&continuation(ROLLOUT_A), &paged(OTHER, Some(OTHER), &[]));
    // `payload.session_id` naming another thread.
    let split = case(
        &continuation(ROLLOUT_B),
        &paged(CODEX, Some(CODEX), &[]).replacen(
            &format!("\"session_id\":\"{CODEX}\""),
            &format!("\"session_id\":\"{OTHER}\""),
            1,
        ),
    );
    // A `history_base` that names no rollout.
    let malformed = case(
        "rollout-2026-09-08T12-00-01-019a0000-0000-7000-8000-00000000c0de_019a0000-0000-7000-8000-0000000a0003.jsonl",
        &lines(&[json!({"type": "session_meta", "payload": {"id": CODEX,
                         "history_mode": "paginated", "history_base": "root"}})
        .to_string()]),
    );
    // A continuation's name over a flat header, and a header that is not the
    // opening line.
    let flat = case(
        "rollout-2026-09-08T12-00-02-019a0000-0000-7000-8000-00000000c0de_019a0000-0000-7000-8000-0000000a0004.jsonl",
        &rollout(CODEX, &[]),
    );
    let late = case(
        "rollout-2026-09-08T12-00-03-019a0000-0000-7000-8000-00000000c0de_019a0000-0000-7000-8000-0000000a0005.jsonl",
        &lines(&[
            json!({"type": "turn_context", "payload": {}}).to_string(),
            paged_header(CODEX, Some(CODEX)),
        ]),
    );
    for (path, why) in [
        (&foreign, Untitled::IdentityMismatch),
        (&split, Untitled::IdentityMismatch),
        (&malformed, Untitled::IdentityMismatch),
        (&flat, Untitled::IdentityMismatch),
        (&late, Untitled::IdentityMismatch),
    ] {
        assert_eq!(
            read(&home, &[codex(CODEX, &[path])]),
            [TitleOutcome::Untitled(why)],
            "{}",
            path.display()
        );
        assert_eq!(
            read(&home, &[codex(CODEX, &[&root, path])]),
            [TitleOutcome::Untitled(why)],
            "with the root: {}",
            path.display()
        );
    }

    // Two roots of one thread: the same rollout identity twice.
    let twin = home.join(format!(
        ".codex/sessions/2026/09/10/rollout-2026-09-10T12-00-00-{CODEX}.jsonl"
    ));
    write(&twin, &paged(CODEX, None, &[]));
    // Two files of one continuation, in different days.
    let part = continuation_path(&home, "11", CODEX, ROLLOUT_B);
    let part_twin = continuation_path(&home, "12", CODEX, ROLLOUT_B);
    write(&part, &paged(CODEX, Some(CODEX), &[]));
    write(&part_twin, &paged(CODEX, Some(CODEX), &[]));
    // A second original: a continuation that continues from nothing.
    let second_original = continuation_path(&home, "13", CODEX, ROLLOUT_A);
    write(&second_original, &paged(CODEX, None, &[]));
    // A flat rollout of the thread beside the paginated root.
    let flat_root = home.join(format!(
        ".codex/sessions/2026/09/14/rollout-2026-09-14T12-00-00-{CODEX}.jsonl"
    ));
    write(&flat_root, &rollout(CODEX, &[thread_name(CODEX, "Flat")]));
    for (paths, label) in [
        (vec![&root, &twin], "two roots"),
        (vec![&root, &part, &part_twin], "one rollout twice"),
        (vec![&root, &second_original], "two originals"),
        (vec![&root, &flat_root], "flat beside paginated"),
    ] {
        assert_eq!(
            read(&home, &[codex(CODEX, &paths)]),
            [TitleOutcome::Untitled(Untitled::Ambiguous)],
            "{label}"
        );
    }
}

/// A missing old locator is tolerated only beside a live verified one; a
/// sidecar row alone never names a thread. An alias among the recorded files,
/// or a root replaced by another thread's file, leaves the identifier.
#[test]
fn missing_aliased_or_replaced_paginated_sources_leave_the_identifier() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    sidecar(&home, &[(CODEX, "Thread name")]);
    let gone = rollout_path(&home, CODEX);
    let part = continuation_path(&home, "08", CODEX, ROLLOUT_A);
    write(&part, &paged(CODEX, Some(CODEX), &[]));
    assert_eq!(
        read(&home, &[codex(CODEX, &[&gone, &part])]),
        [titled("Thread name", TitleSource::CodexSidecar)]
    );
    let also_gone = continuation_path(&home, "09", CODEX, ROLLOUT_B);
    assert_eq!(
        read(&home, &[codex(CODEX, &[&gone, &also_gone])]),
        [TitleOutcome::Untitled(Untitled::Missing)]
    );

    // An alias at a continuation's name, to a real continuation elsewhere.
    let real = temp.path().join("elsewhere.jsonl");
    write(&real, &paged(CODEX, Some(ROLLOUT_A), &[]));
    let aliased = continuation_path(&home, "09", CODEX, ROLLOUT_B);
    fs::create_dir_all(aliased.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, &aliased).unwrap();
    for paths in [vec![&aliased], vec![&part, &aliased]] {
        assert_eq!(
            read(&home, &[codex(CODEX, &paths)]),
            [TitleOutcome::Untitled(Untitled::Outside)]
        );
    }

    // The root replaced by another thread's rollout at the same name.
    let root = rollout_path(&home, CODEX);
    write(&root, &paged(CODEX, None, &[]));
    let staged = root.with_extension("staged");
    write(&staged, &paged(OTHER, None, &[]));
    fs::rename(&staged, &root).unwrap();
    assert_eq!(
        read(&home, &[codex(CODEX, &[&root, &part])]),
        [TitleOutcome::Untitled(Untitled::IdentityMismatch)]
    );
}

/// A flat root names its thread only as the one locator the index recorded
/// for it. Beside any other recorded file — gone, an alias, or outside the
/// root — the root's rename may be one a later segment superseded, so neither
/// it nor the sidecar is shown. A paginated root beside a gone locator is
/// still named by its sidecar row.
#[test]
fn a_flat_root_beside_another_recorded_locator_leaves_the_identifier() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    sidecar(&home, &[(CODEX, "Sidecar name")]);
    let root = rollout_path(&home, CODEX);
    write(&root, &rollout(CODEX, &[thread_name(CODEX, "Root rename")]));
    assert_eq!(
        read(&home, &[codex(CODEX, &[&root])]),
        [titled("Root rename", TitleSource::CodexRollout)],
        "a single flat root keeps its rename ahead of the sidecar"
    );

    let gone = continuation_path(&home, "08", CODEX, ROLLOUT_A);
    let real = temp.path().join("elsewhere.jsonl");
    write(&real, &paged(CODEX, Some(CODEX), &[]));
    let aliased = continuation_path(&home, "09", CODEX, ROLLOUT_B);
    fs::create_dir_all(aliased.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, &aliased).unwrap();
    let outside = temp.path().join(format!(
        "elsewhere/rollout-2026-09-09T12-00-00-{CODEX}_{ROLLOUT_B}.jsonl"
    ));
    write(&outside, &paged(CODEX, Some(CODEX), &[]));
    for (other, label) in [
        (&gone, "missing continuation"),
        (&aliased, "aliased continuation"),
        (&outside, "outside continuation"),
    ] {
        for paths in [vec![&root, other], vec![other, &root]] {
            let batch = read_titles(&home, &[codex(CODEX, &paths)], TitleLimits::default(), None);
            assert_eq!(
                batch.outcomes,
                [TitleOutcome::Untitled(Untitled::Ambiguous)],
                "{label}"
            );
            // Only the root's opening line was probed; nothing was streamed
            // and the sidecar was not read.
            assert!(
                batch.bytes_read <= 16 * 1024,
                "{label}: {} bytes read",
                batch.bytes_read
            );
        }
    }
    // A flat root with no rename of its own is not named by the sidecar either.
    write(&root, &rollout(CODEX, &[]));
    assert_eq!(
        read(&home, &[codex(CODEX, &[&root, &gone])]),
        [TitleOutcome::Untitled(Untitled::Ambiguous)]
    );
    // Its lone flat root is still named by the sidecar, as before.
    assert_eq!(
        read(&home, &[codex(CODEX, &[&root])]),
        [titled("Sidecar name", TitleSource::CodexSidecar)]
    );

    // A paginated root beside the same gone locator keeps the sidecar lane.
    write(
        &root,
        &paged(CODEX, None, &[thread_name(CODEX, "Root rename")]),
    );
    assert_eq!(
        read(&home, &[codex(CODEX, &[&root, &gone])]),
        [titled("Sidecar name", TitleSource::CodexSidecar)]
    );
}

/// An opening line longer than the inspected bound, torn, or absent proves no
/// thread.
#[test]
fn an_overlong_torn_or_empty_paginated_header_leaves_the_identifier() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    sidecar(&home, &[(CODEX, "Thread name")]);
    let part = continuation_path(&home, "08", CODEX, ROLLOUT_A);
    let target = [codex(CODEX, &[&part])];
    let limits = TitleLimits {
        max_line_bytes: 2048,
        ..TitleLimits::default()
    };
    let header = paged_header(CODEX, Some(CODEX));
    assert!(header.len() < 2048);
    let long = header.replace("long instructions ", &"long instructions ".repeat(4));
    assert!(long.len() > 2048);
    for (body, why) in [
        (lines(std::slice::from_ref(&long)), Untitled::LineTooLong),
        // Overlong and torn: still longer than the bound.
        (long, Untitled::LineTooLong),
        (header.clone(), Untitled::IdentityMismatch),
        (String::new(), Untitled::IdentityMismatch),
    ] {
        write(&part, &body);
        assert_eq!(
            read_titles(&home, &target, limits, None).outcomes,
            [TitleOutcome::Untitled(why)]
        );
    }
    write(&part, &lines(&[header]));
    assert_eq!(
        read_titles(&home, &target, limits, None).outcomes,
        [titled("Thread name", TitleSource::CodexSidecar)]
    );
}

/// The paginated lane needs an intact exact-ID row: no sidecar, a wrong ID, a
/// torn last row, an overlong row or an aliased sidecar names nothing, and a
/// torn last row does not hide the complete row before it.
#[test]
fn the_paginated_lane_needs_an_intact_exact_sidecar_row() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let root = rollout_path(&home, CODEX);
    write(&root, &paged(CODEX, None, &[]));
    let target = [codex(CODEX, &[&root])];
    let limits = TitleLimits {
        max_line_bytes: 2048,
        ..TitleLimits::default()
    };
    let row = |id: &str, name: &str| json!({"id": id, "thread_name": name}).to_string();
    let path = home.join(".codex/session_index.jsonl");
    let untitled = [TitleOutcome::Untitled(Untitled::NoTitle)];
    assert_eq!(read_titles(&home, &target, limits, None).outcomes, untitled);
    for body in [
        lines(&[
            row(OTHER, "Wrong thread"),
            row(&CODEX.to_uppercase(), "Case"),
        ]),
        row(CODEX, "Torn"),
        lines(&[row(CODEX, "Early"), row(OTHER, &"x".repeat(4096))]),
    ] {
        write(&path, &body);
        assert_eq!(
            read_titles(&home, &target, limits, None).outcomes,
            untitled,
            "{}",
            body.len()
        );
    }
    let mut body = lines(&[row(CODEX, "Complete")]);
    body.push_str(&row(CODEX, "Torn after"));
    write(&path, &body);
    assert_eq!(
        read_titles(&home, &target, limits, None).outcomes,
        [titled("Complete", TitleSource::CodexSidecar)]
    );
    let real = temp.path().join("index.jsonl");
    write(&real, &lines(&[row(CODEX, "Aliased")]));
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&real, &path).unwrap();
    assert_eq!(read_titles(&home, &target, limits, None).outcomes, untitled);
}

/// More recorded files than a title read verifies, or headers beyond the
/// batch's budget, leave the identifier; so does a cancel or the deadline.
#[test]
fn over_cap_over_budget_cancelled_or_late_paginated_reads_leave_the_identifier() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    sidecar(&home, &[(CODEX, "Thread name")]);
    let root = rollout_path(&home, CODEX);
    write(&root, &paged(CODEX, None, &[]));
    let mut paths = vec![root.clone()];
    for index in 1..=MAX_CODEX_LOCATORS {
        let rollout = format!("019a0000-0000-7000-8000-0000000b{index:04}");
        let path = continuation_path(&home, "08", CODEX, &rollout);
        write(&path, &paged(CODEX, Some(CODEX), &[]));
        paths.push(path);
    }
    let all: Vec<&PathBuf> = paths.iter().collect();
    let batch = read_titles(&home, &[codex(CODEX, &all)], TitleLimits::default(), None);
    assert_eq!(
        batch.outcomes,
        [TitleOutcome::Untitled(Untitled::TooManySources)]
    );
    assert_eq!(batch.bytes_read, 0, "nothing is opened past the cap");
    let capped = codex(CODEX, &all[..MAX_CODEX_LOCATORS]);
    assert_eq!(
        read(&home, std::slice::from_ref(&capped)),
        [titled("Thread name", TitleSource::CodexSidecar)]
    );

    let header = fs::metadata(&root).unwrap().len();
    let short = TitleLimits {
        max_batch_bytes: 2 * header,
        ..TitleLimits::default()
    };
    assert_eq!(
        read_titles(&home, std::slice::from_ref(&capped), short, None).outcomes,
        [TitleOutcome::Untitled(Untitled::Budget)]
    );

    let cancel = CancelToken::new();
    cancel.cancel();
    let batch = read_titles(
        &home,
        std::slice::from_ref(&capped),
        TitleLimits::default(),
        Some(&cancel),
    );
    assert_eq!(
        batch.outcomes,
        [TitleOutcome::Untitled(Untitled::Cancelled)]
    );
    assert_eq!(batch.bytes_read, 0);
    let late = TitleLimits {
        deadline: Duration::ZERO,
        ..TitleLimits::default()
    };
    let batch = read_titles(&home, &[capped], late, None);
    assert_eq!(batch.outcomes, [TitleOutcome::Untitled(Untitled::Deadline)]);
    assert_eq!(batch.bytes_read, 0);
}

/// A thread's root and continuations are found by exact name, and no more of
/// them than a read could use; a near miss — another thread's continuation
/// whose rollout identity is this thread's, a non-UUID rollout, a double
/// suffix — is never this thread's.
#[test]
fn codex_title_sources_find_a_threads_root_and_continuations() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let mut db = TempDb::empty().unwrap();
    let record = |db: &mut TempDb, path: &Path| {
        db.store_mut()
            .record_native_source_locator(
                &SourceCursor {
                    source: SessionSource::ReadersCli,
                    cursor_key: format!("codex:{}", path.display()),
                    position: 0,
                    updated_at: OBSERVED_AT,
                },
                true,
            )
            .unwrap();
    };
    let root = rollout_path(&home, CODEX);
    let part = continuation_path(&home, "08", CODEX, ROLLOUT_A);
    let day = home.join(".codex/sessions/2026/09/08");
    for path in [
        root.clone(),
        part.clone(),
        continuation_path(&home, "08", OTHER, CODEX),
        continuation_path(&home, "08", PARENT, CODEX),
        day.join(format!(
            "rollout-2026-09-08T12-00-00-{CODEX}_not-a-uuid.jsonl"
        )),
        day.join(format!(
            "rollout-2026-09-08T12-00-00-{CODEX}_{ROLLOUT_A}_{ROLLOUT_B}.jsonl"
        )),
        day.join(format!("rollout-{CODEX}_{ROLLOUT_B}.jsonl")),
        day.join(format!(
            "rollout-2026-09-08T12-00-00-{CODEX}_{ROLLOUT_B}.jsonl.bak"
        )),
    ] {
        record(&mut db, &path);
    }
    let found: Vec<String> = title_sources(db.store(), Host::Codex, CODEX)
        .unwrap()
        .into_iter()
        .map(|source| source.locator)
        .collect();
    let mut expected = vec![
        format!("codex:{}", root.display()),
        format!("codex:{}", part.display()),
    ];
    expected.sort();
    assert_eq!(found, expected);

    for index in 0..2 * MAX_CODEX_LOCATORS {
        let rollout = format!("019a0000-0000-7000-8000-0000000c{index:04}");
        record(&mut db, &continuation_path(&home, "09", CODEX, &rollout));
    }
    assert_eq!(
        title_sources(db.store(), Host::Codex, CODEX).unwrap().len(),
        MAX_CODEX_LOCATORS + 1,
        "one past the cap, so a read sees the thread has too many"
    );
}

fn record_codex(db: &mut TempDb, path: &Path) {
    db.store_mut()
        .record_native_source_locator(
            &SourceCursor {
                source: SessionSource::ReadersCli,
                cursor_key: format!("codex:{}", path.display()),
                position: 0,
                updated_at: OBSERVED_AT,
            },
            true,
        )
        .unwrap();
}

/// A key the locator patterns match that no exact name check accepts: a
/// continuation of this thread whose rollout is not a UUID, in a day that
/// sorts before every real file of the thread.
fn near_miss(home: &Path, index: usize) -> PathBuf {
    home.join(".codex/sessions/2026/09/06").join(format!(
        "rollout-2026-09-06T12-00-00-{CODEX}_x{index:04}.jsonl"
    ))
}

/// Near misses that sort ahead of a thread's real locators do not hide them,
/// up to the index lookup's row ceiling; one row past it, the thread is named
/// by nothing — not by a prefix of what matched — and no file is opened. The
/// other row of the batch is unaffected.
#[test]
fn codex_locator_lookup_is_bounded_before_names_are_checked() {
    const OTHER_CODEX: &str = "019a0000-0000-7000-8000-00000000d0de";
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    sidecar(
        &home,
        &[(CODEX, "Thread name"), (OTHER_CODEX, "Other thread")],
    );
    let root = rollout_path(&home, CODEX);
    let part = continuation_path(&home, "08", CODEX, ROLLOUT_A);
    write(&root, &paged(CODEX, None, &[]));
    write(&part, &paged(CODEX, Some(CODEX), &[]));
    let other = rollout_path(&home, OTHER_CODEX);
    write(&other, &rollout(OTHER_CODEX, &[]));
    let mut db = TempDb::empty().unwrap();
    for path in [&root, &part, &other] {
        record_codex(&mut db, path);
    }
    // Far more near misses than an unbounded prefix of nine could see past,
    // yet the thread's own rows still fit under the ceiling.
    let fitting = MAX_LOCATOR_ROWS - 2;
    for index in 0..fitting {
        record_codex(&mut db, &near_miss(&home, index));
    }
    let targets = |db: &TempDb| {
        [CODEX, OTHER_CODEX].map(|native| {
            target(
                Host::Codex,
                native,
                title_sources(db.store(), Host::Codex, native).unwrap(),
            )
        })
    };
    let found = targets(&db);
    assert_eq!(
        found[0].locators,
        vec![codex_locator(&root), codex_locator(&part)]
    );
    assert_eq!(
        read(&home, &found),
        [
            titled("Thread name", TitleSource::CodexSidecar),
            titled("Other thread", TitleSource::CodexSidecar),
        ]
    );

    // One row past the ceiling, though fewer than nine are exact: no locator
    // is named, so the thread is read from no file.
    record_codex(&mut db, &near_miss(&home, fitting));
    let found = targets(&db);
    assert!(found[0].locators.is_empty());
    assert_eq!(found[1].locators, vec![codex_locator(&other)]);
    let alone = read_titles(&home, &found[..1], TitleLimits::default(), None);
    assert_eq!(
        alone.outcomes,
        [TitleOutcome::Untitled(Untitled::NotIndexed)]
    );
    assert_eq!(alone.bytes_read, 0);
    assert_eq!(
        read(&home, &found),
        [
            TitleOutcome::Untitled(Untitled::NotIndexed),
            titled("Other thread", TitleSource::CodexSidecar),
        ]
    );
}

/// More exact locators than a read verifies are one past the cap from the
/// index, and the read declines the thread before opening any of them.
#[test]
fn more_exact_codex_locators_than_the_cap_open_no_file() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    sidecar(&home, &[(CODEX, "Thread name")]);
    let mut db = TempDb::empty().unwrap();
    let root = rollout_path(&home, CODEX);
    write(&root, &paged(CODEX, None, &[]));
    record_codex(&mut db, &root);
    for index in 0..MAX_CODEX_LOCATORS {
        let rollout = format!("019a0000-0000-7000-8000-0000000c{index:04}");
        let path = continuation_path(&home, "09", CODEX, &rollout);
        write(&path, &paged(CODEX, Some(CODEX), &[]));
        record_codex(&mut db, &path);
    }
    let locators = title_sources(db.store(), Host::Codex, CODEX).unwrap();
    assert_eq!(locators.len(), MAX_CODEX_LOCATORS + 1);
    let batch = read_titles(
        &home,
        &[target(Host::Codex, CODEX, locators)],
        TitleLimits::default(),
        None,
    );
    assert_eq!(
        batch.outcomes,
        [TitleOutcome::Untitled(Untitled::TooManySources)]
    );
    assert_eq!(batch.bytes_read, 0);
}

/// A paginated read writes nothing: the rollouts, the sidecar and the index are
/// byte-for-byte unchanged, and the tree holds no new file.
#[test]
fn a_paginated_title_read_changes_no_source_and_no_stored_row() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let root = rollout_path(&home, CODEX);
    let part = continuation_path(&home, "08", CODEX, ROLLOUT_A);
    write(&root, &paged(CODEX, None, &[]));
    write(&part, &paged(CODEX, Some(CODEX), &[]));
    let index = sidecar(&home, &[(CODEX, "Kept in memory")]);
    let mut db = TempDb::empty().unwrap();
    for path in [&root, &part] {
        db.store_mut()
            .record_native_source_locator(
                &SourceCursor {
                    source: SessionSource::ReadersCli,
                    cursor_key: format!("codex:{}", path.display()),
                    position: 0,
                    updated_at: OBSERVED_AT,
                },
                true,
            )
            .unwrap();
    }
    let sources: Vec<Vec<u8>> = [&root, &part, &index]
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect();
    let before_db = fs::read(db.path()).unwrap();
    let before_listing = listing(&home);
    let targets = [target(
        Host::Codex,
        CODEX,
        title_sources(db.store(), Host::Codex, CODEX).unwrap(),
    )];
    assert_eq!(
        read(&home, &targets),
        [titled("Kept in memory", TitleSource::CodexSidecar)]
    );
    for (path, before) in [&root, &part, &index].iter().zip(&sources) {
        assert_eq!(&fs::read(path).unwrap(), before);
    }
    assert_eq!(fs::read(db.path()).unwrap(), before_db);
    assert_eq!(listing(&home), before_listing);
}
