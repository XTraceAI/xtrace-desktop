//! Looking one session's recorded source locators up by key pattern. The
//! lookup reads `source_cursors` only: it answers where the index read a
//! session from, never what the session said. Wildcards inside a supplied
//! identifier match themselves, and the pattern count is bounded. A bounded
//! lookup returns every match up to its row ceiling, or says it saturated.
use xt_fixtures::TempDb;
use xt_store::{
    SessionSource, Store,
    batch::{LocatorRows, MAX_LOCATOR_PATTERNS, MAX_LOCATOR_ROWS, SourceCursor, escape_like},
};

fn locator(source: SessionSource, key: &str) -> SourceCursor {
    SourceCursor {
        source,
        cursor_key: key.into(),
        position: 0,
        updated_at: 1_788_782_400_000,
    }
}

fn record(store: &mut Store, source: SessionSource, key: &str) {
    store
        .record_native_source_locator(&locator(source, key), true)
        .unwrap();
}

fn keys(store: &Store, source: SessionSource, patterns: &[&str]) -> Vec<String> {
    store
        .source_cursors_like(source, patterns)
        .unwrap()
        .into_iter()
        .map(|cursor| cursor.cursor_key)
        .collect()
}

#[test]
fn a_session_matches_its_own_transcript_and_subagent_locators_only() {
    let mut db = TempDb::empty().unwrap();
    let session = "00000000-0000-4000-8000-00000000aaaa";
    let other = "00000000-0000-4000-8000-00000000bbbb";
    let transcript = format!("claude:/home/.claude/projects/repo/{session}.jsonl");
    let subagent = format!("claude:/home/.claude/projects/repo/{session}/subagents/a/1.jsonl");
    let unrelated = format!("claude:/home/.claude/projects/repo/{other}.jsonl");
    for key in [&transcript, &subagent, &unrelated] {
        record(db.store_mut(), SessionSource::Transcript, key);
    }
    // Another host's locator lives under another source and cannot be reached
    // by a transcript lookup even when its key names the same identifier.
    record(
        db.store_mut(),
        SessionSource::ReadersCli,
        &format!("codex:/home/.codex/sessions/{session}.jsonl"),
    );
    let escaped = escape_like(session);
    let patterns = [
        format!("claude:%/{escaped}.jsonl"),
        format!("claude:%/{escaped}/subagents/%"),
    ];
    let patterns: Vec<&str> = patterns.iter().map(String::as_str).collect();
    assert_eq!(
        keys(db.store(), SessionSource::Transcript, &patterns),
        vec![transcript, subagent]
    );
    assert!(!keys(db.store(), SessionSource::Transcript, &patterns).contains(&unrelated));
}

#[test]
fn wildcards_inside_an_identifier_match_themselves() {
    let mut db = TempDb::empty().unwrap();
    // `_` and `%` are LIKE wildcards; escaped, they are ordinary characters,
    // so one odd identifier cannot borrow another session's locator.
    let odd = "a_b%c";
    let decoy = "axbyc";
    for id in [odd, decoy] {
        record(
            db.store_mut(),
            SessionSource::Transcript,
            &format!("claude:/home/.claude/projects/repo/{id}.jsonl"),
        );
    }
    let escaped = escape_like(odd);
    let matched = keys(
        db.store(),
        SessionSource::Transcript,
        &[&format!("claude:%/{escaped}.jsonl")],
    );
    assert_eq!(
        matched,
        vec![format!("claude:/home/.claude/projects/repo/{odd}.jsonl")]
    );
    // Unescaped, the same identifier would also claim the decoy's locator.
    assert_eq!(
        keys(
            db.store(),
            SessionSource::Transcript,
            &[&format!("claude:%/{odd}.jsonl")],
        )
        .len(),
        2
    );
}

#[test]
fn a_lookup_needs_at_least_one_pattern_and_no_more_than_the_bound() {
    let db = TempDb::empty().unwrap();
    assert!(
        db.store()
            .source_cursors_like(SessionSource::Transcript, &[])
            .is_err()
    );
    let many = vec!["claude:%"; MAX_LOCATOR_PATTERNS + 1];
    assert!(
        db.store()
            .source_cursors_like(SessionSource::Transcript, &many)
            .is_err()
    );
    assert!(
        db.store()
            .source_cursors_like(SessionSource::Transcript, &many[..MAX_LOCATOR_PATTERNS])
            .is_ok()
    );
}

#[test]
fn a_session_the_index_never_recorded_has_no_locator() {
    let db = TempDb::empty().unwrap();
    assert!(
        keys(
            db.store(),
            SessionSource::Transcript,
            &["claude:%/absent.jsonl"],
        )
        .is_empty()
    );
}

/// A bounded lookup returns every match up to its ceiling, in key order, and
/// one more match makes it saturate rather than return a prefix. Rows of
/// another source or matching no pattern are not counted against it.
#[test]
fn a_bounded_lookup_returns_every_match_or_saturates() {
    let mut db = TempDb::empty().unwrap();
    let key = |index: usize| format!("codex:/home/.codex/sessions/rollout-{index:04}-t.jsonl");
    for index in 0..MAX_LOCATOR_ROWS {
        record(db.store_mut(), SessionSource::ReadersCli, &key(index));
        record(db.store_mut(), SessionSource::Transcript, &key(index));
    }
    record(
        db.store_mut(),
        SessionSource::ReadersCli,
        "codex:/home/.codex/sessions/rollout-9999-u.jsonl",
    );
    let bounded = |store: &Store, patterns: &[&str]| {
        store
            .source_cursors_like_bounded(SessionSource::ReadersCli, patterns)
            .unwrap()
    };
    let LocatorRows::Complete(rows) = bounded(db.store(), &["codex:%-t.jsonl"]) else {
        panic!("exactly the ceiling is complete");
    };
    let found: Vec<String> = rows.into_iter().map(|row| row.cursor_key).collect();
    assert_eq!(found.len(), MAX_LOCATOR_ROWS);
    assert_eq!(
        found,
        keys(db.store(), SessionSource::ReadersCli, &["codex:%-t.jsonl"])
    );

    record(
        db.store_mut(),
        SessionSource::ReadersCli,
        &key(MAX_LOCATOR_ROWS),
    );
    assert_eq!(
        bounded(db.store(), &["codex:%-t.jsonl"]),
        LocatorRows::Saturated
    );
    // Two patterns share one ceiling.
    assert_eq!(
        bounded(db.store(), &["codex:%-0000-t.jsonl", "codex:%-t.jsonl"]),
        LocatorRows::Saturated
    );
    assert_eq!(
        bounded(db.store(), &["codex:%-u.jsonl"]),
        LocatorRows::Complete(vec![locator(
            SessionSource::ReadersCli,
            "codex:/home/.codex/sessions/rollout-9999-u.jsonl",
        )])
    );
    assert!(
        db.store()
            .source_cursors_like_bounded(SessionSource::ReadersCli, &[])
            .is_err()
    );
    assert!(
        db.store()
            .source_cursors_like_bounded(
                SessionSource::ReadersCli,
                &["codex:%"; MAX_LOCATOR_PATTERNS + 1],
            )
            .is_err()
    );
}
