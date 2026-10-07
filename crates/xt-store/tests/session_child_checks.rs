//! The display check of each indexed session: checking from its first
//! committed records, checked only for the attempt and own-check key a
//! settlement captured under today's detector versions, unchanged by ordinary
//! later messages, moved on by changed own-check inputs, a failed observation
//! or a launch newly naming exactly that session. Positive child evidence
//! stays independent of it.

use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    batch::IngestBatch,
    child_check::{ChildCheck, DETECTOR_VERSIONS},
    child_fact::{ChildEvidence, ChildFact},
    claude_launch::{GroupStatus, LaunchCandidate, LaunchKey, MemberRecord, SegmentGeneration},
    ingest::DiscoveredSession,
    session_list,
};

const MAIN: &str = "019a0000-0000-7000-8000-00000000aa01";
const CHILD: &str = "019a0000-0000-7000-8000-00000000aa02";
const CALLER: &str = "019a0000-0000-7000-8000-00000000aa03";
const CLAUDE: &str = "0c000000-0000-4000-8000-00000000aa04";
const KEY_A: &str = "x1:0000000000000000000000000000000000000000000000000000000000000001";
const KEY_B: &str = "x1:0000000000000000000000000000000000000000000000000000000000000002";

fn id(native: &str) -> String {
    format!("codex-{native}")
}

fn path(directory: &TempDir) -> std::path::PathBuf {
    directory.path().join("checks.sqlite")
}

fn sql(directory: &TempDir) -> Connection {
    Connection::open(path(directory)).unwrap()
}

fn record(native: &str, n: u32) -> CanonicalRecord {
    serde_json::from_value(json!({
        "uuid": format!("r{n}-{native}"), "type": "user",
        "timestamp": format!("2026-09-01T00:00:{:02}Z", n),
        "message": {"role": "user", "content": [{"type": "text", "text": "Synthetic"}]}
    }))
    .unwrap()
}

fn session(store: &mut Store, native: &str) {
    let mut meta = SessionMeta::new(id(native), "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some(native.into());
    store.upsert_session(&meta, false).unwrap();
}

fn check(directory: &TempDir, native: &str) -> ChildCheck {
    session_list::context(&sql(directory), &[&id(native)])
        .unwrap()
        .remove(0)
        .check
}

#[test]
fn a_session_is_checking_until_its_captured_attempt_and_key_settle() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(path(&directory)).unwrap();
    session(&mut store, MAIN);
    // No records committed yet: no row, and the read fails closed.
    assert_eq!(check(&directory, MAIN), ChildCheck::Checking);
    store
        .upsert_records(&id(MAIN), &[record(MAIN, 1)], false)
        .unwrap();
    assert_eq!(check(&directory, MAIN), ChildCheck::Checking);
    let attempt = store.child_check_attempt(&id(MAIN)).unwrap().unwrap();
    assert_eq!((attempt.required, attempt.own_check_key), (1, None));
    // No key observed: nothing settles.
    assert!(store.settle_child_checks(&[(&id(MAIN), 1, KEY_A)]).unwrap() == 0);
    // A first key is kept without moving the attempt.
    let observed = store
        .observe_own_check(&id(MAIN), Some(KEY_A))
        .unwrap()
        .unwrap();
    assert_eq!(observed.required, 1);
    // A settlement captured with another key or attempt is refused.
    assert_eq!(
        store.settle_child_checks(&[(&id(MAIN), 1, KEY_B)]).unwrap(),
        0
    );
    assert_eq!(
        store.settle_child_checks(&[(&id(MAIN), 2, KEY_A)]).unwrap(),
        0
    );
    assert_eq!(check(&directory, MAIN), ChildCheck::Checking);
    assert_eq!(
        store.settle_child_checks(&[(&id(MAIN), 1, KEY_A)]).unwrap(),
        1
    );
    assert_eq!(check(&directory, MAIN), ChildCheck::Checked);
    // Settling again changes nothing, and says so.
    assert_eq!(
        store.settle_child_checks(&[(&id(MAIN), 1, KEY_A)]).unwrap(),
        0
    );
}

#[test]
fn ordinary_messages_and_the_same_key_keep_a_checked_session_checked() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(path(&directory)).unwrap();
    session(&mut store, MAIN);
    store
        .upsert_records(&id(MAIN), &[record(MAIN, 1)], false)
        .unwrap();
    store.observe_own_check(&id(MAIN), Some(KEY_A)).unwrap();
    store.settle_child_checks(&[(&id(MAIN), 1, KEY_A)]).unwrap();
    for n in 2..6 {
        store
            .upsert_records(&id(MAIN), &[record(MAIN, n)], false)
            .unwrap();
        // A reader observing the same inputs again, as a restart does.
        store.observe_own_check(&id(MAIN), Some(KEY_A)).unwrap();
        assert_eq!(check(&directory, MAIN), ChildCheck::Checked, "message {n}");
    }
    assert_eq!(
        store
            .child_check_attempt(&id(MAIN))
            .unwrap()
            .unwrap()
            .required,
        1
    );
}

#[test]
fn native_first_input_changes_reopen_in_the_record_commit() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(path(&directory)).unwrap();
    let mut meta = SessionMeta::new(id(MAIN), "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some(MAIN.into());
    let discovery = DiscoveredSession {
        host: Host::Codex,
        native_session_id: MAIN.into(),
        conversation_id: Some(id(MAIN)),
        surface: None,
        started_at_ms: None,
        last_observed_at: 1,
        discovery_complete: true,
    };
    let first = record(MAIN, 1);
    let mut batch = IngestBatch::new(&meta, std::slice::from_ref(&first), false);
    batch.native_codex = true;
    batch.discovery = Some(&discovery);
    store.apply_ingest_batch(&batch).unwrap();
    store.observe_own_check(&id(MAIN), Some(KEY_A)).unwrap();
    store.settle_child_checks(&[(&id(MAIN), 1, KEY_A)]).unwrap();
    assert_eq!(check(&directory, MAIN), ChildCheck::Checked);

    let later = record(MAIN, 2);
    let mut batch = IngestBatch::new(&meta, std::slice::from_ref(&later), false);
    batch.native_codex = true;
    batch.discovery = Some(&discovery);
    store.apply_ingest_batch(&batch).unwrap();
    assert_eq!(check(&directory, MAIN), ChildCheck::Checked);

    let earlier = record(MAIN, 0);
    let mut batch = IngestBatch::new(&meta, std::slice::from_ref(&earlier), false);
    batch.native_codex = true;
    batch.discovery = Some(&discovery);
    store.apply_ingest_batch(&batch).unwrap();
    assert_eq!(check(&directory, MAIN), ChildCheck::Checking);
    assert_eq!(
        store
            .child_check_attempt(&id(MAIN))
            .unwrap()
            .unwrap()
            .required,
        2
    );
    assert_eq!(
        store.settle_child_checks(&[(&id(MAIN), 1, KEY_A)]).unwrap(),
        0
    );
}

#[test]
fn changed_or_unreadable_own_inputs_reopen_only_that_session() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(path(&directory)).unwrap();
    for native in [MAIN, CALLER] {
        session(&mut store, native);
        store
            .upsert_records(&id(native), &[record(native, 1)], false)
            .unwrap();
        store.observe_own_check(&id(native), Some(KEY_A)).unwrap();
        store
            .settle_child_checks(&[(&id(native), 1, KEY_A)])
            .unwrap();
    }
    // A rewritten opening: a new attempt, and the old settlement no longer fits.
    let moved = store
        .observe_own_check(&id(MAIN), Some(KEY_B))
        .unwrap()
        .unwrap();
    assert_eq!(moved.required, 2);
    assert_eq!(check(&directory, MAIN), ChildCheck::Checking);
    assert_eq!(check(&directory, CALLER), ChildCheck::Checked);
    assert_eq!(
        store.settle_child_checks(&[(&id(MAIN), 1, KEY_A)]).unwrap(),
        0
    );
    assert_eq!(
        store.settle_child_checks(&[(&id(MAIN), 2, KEY_B)]).unwrap(),
        1
    );
    // A reader that could not observe the inputs proves nothing unchanged.
    let failed = store.observe_own_check(&id(MAIN), None).unwrap().unwrap();
    assert_eq!((failed.required, failed.own_check_key), (3, None));
    assert_eq!(check(&directory, MAIN), ChildCheck::Checking);
    // A second failure moves nothing further; the next read keeps its key.
    assert_eq!(
        store
            .observe_own_check(&id(MAIN), None)
            .unwrap()
            .unwrap()
            .required,
        3
    );
    assert_eq!(
        store
            .observe_own_check(&id(MAIN), Some(KEY_B))
            .unwrap()
            .unwrap()
            .required,
        3
    );
    assert_eq!(check(&directory, CALLER), ChildCheck::Checked);
    // Explicit invalidation of in-flight targets names exactly them.
    assert_eq!(store.invalidate_child_checks(&[&id(CALLER)]).unwrap(), 1);
    assert_eq!(check(&directory, CALLER), ChildCheck::Checking);
    assert_eq!(
        store.invalidate_child_checks_of(Host::Codex, MAIN).unwrap(),
        1
    );
    assert_eq!(
        store
            .child_check_attempt(&id(MAIN))
            .unwrap()
            .unwrap()
            .required,
        4
    );
    // An unknown or non-user session is not given a row.
    assert!(
        store
            .observe_own_check("nobody", Some(KEY_A))
            .unwrap()
            .is_none()
    );
}

#[test]
fn other_detector_versions_read_as_checking_and_children_stay_children() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(path(&directory)).unwrap();
    for native in [MAIN, CHILD] {
        session(&mut store, native);
        store
            .upsert_records(&id(native), &[record(native, 1)], false)
            .unwrap();
        store.observe_own_check(&id(native), Some(KEY_A)).unwrap();
        store
            .settle_child_checks(&[(&id(native), 1, KEY_A)])
            .unwrap();
    }
    sql(&directory)
        .execute(
            "UPDATE session_child_checks SET detector_versions='check0' WHERE session_id=?1",
            [id(MAIN)],
        )
        .unwrap();
    assert_eq!(check(&directory, MAIN), ChildCheck::Checking);
    assert_eq!(store.open_child_checks(None, 10).unwrap().len(), 1);
    // The same attempt and key settle again under today's versions.
    assert_eq!(
        store.settle_child_checks(&[(&id(MAIN), 1, KEY_A)]).unwrap(),
        1
    );
    assert_eq!(check(&directory, MAIN), ChildCheck::Checked);
    // Positive child evidence is a child whatever its check says; a known
    // child is never an open check.
    store
        .record_child_facts(
            &[ChildFact::typed(
                &id(CHILD),
                CHILD,
                ChildEvidence::CodexGuardian,
            )],
            1,
        )
        .unwrap();
    store.invalidate_child_checks(&[&id(CHILD)]).unwrap();
    let context = session_list::context(&sql(&directory), &[&id(CHILD)])
        .unwrap()
        .remove(0);
    assert!(context.known_child);
    assert_eq!(context.check, ChildCheck::Child);
    assert!(store.open_child_checks(None, 10).unwrap().is_empty());
    assert!(DETECTOR_VERSIONS.starts_with("check1."));
}

fn candidate(child: &str, call: &str) -> LaunchCandidate {
    LaunchCandidate {
        key: LaunchKey {
            parent_native_session_id: CALLER.into(),
            rollout_id: CALLER.into(),
            launch_call_id: call.into(),
            launch_operation_index: 0,
        },
        child_native_session_id: child.into(),
        acknowledgment_call_id: call.into(),
        acknowledgment_operation_index: 0,
        process_session_id: None,
        launch_offset: 10,
        acknowledgment_offset: 20,
        launch_ordinal: None,
        acknowledgment_ordinal: None,
        binding_call_offset: None,
        binding_output_offset: None,
        child_host: Host::Claude,
        launch_check_fingerprint: Some("0".repeat(64)),
    }
}

fn publish(store: &mut Store, candidates: &[LaunchCandidate], length: i64) -> usize {
    let allocation = store.begin_claude_launch_validation(CALLER, 10).unwrap();
    assert!(
        store
            .stage_claude_launch_candidates(CALLER, allocation.allocated, candidates)
            .unwrap()
    );
    let member = MemberRecord {
        rollout_id: CALLER.into(),
        generation: SegmentGeneration {
            device: 1,
            inode: 2,
            length,
            mtime_ns: length,
            ctime_ns: length,
        },
        scanned_length: length,
        valid: true,
    };
    store
        .publish_claude_launch_validation_with_reopened(
            CALLER,
            allocation,
            GroupStatus::Valid,
            &[member],
            candidates.len(),
            0,
        )
        .unwrap()
        .unwrap()
}

#[test]
fn a_launch_newly_naming_a_session_reopens_only_that_session() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(path(&directory)).unwrap();
    let mut meta = SessionMeta::new(CLAUDE, "claude", SessionSource::Transcript);
    meta.native_session_id = Some(CLAUDE.into());
    store.upsert_session(&meta, false).unwrap();
    store
        .upsert_records(CLAUDE, &[record(CLAUDE, 1)], false)
        .unwrap();
    session(&mut store, MAIN);
    store
        .upsert_records(&id(MAIN), &[record(MAIN, 1)], false)
        .unwrap();
    for session in [CLAUDE.to_owned(), id(MAIN)] {
        store.observe_own_check(&session, Some(KEY_A)).unwrap();
        store.settle_child_checks(&[(&session, 1, KEY_A)]).unwrap();
    }
    let read = |native: &str| {
        session_list::context(&sql(&directory), &[native])
            .unwrap()
            .remove(0)
            .check
    };
    // A validation with no launch naming anyone changes nothing.
    assert_eq!(publish(&mut store, &[], 100), 0);
    assert_eq!(read(CLAUDE), ChildCheck::Checked);
    // A launch naming the Claude session: only it reopens, in that commit.
    assert_eq!(publish(&mut store, &[candidate(CLAUDE, "call_a")], 200), 1);
    assert_eq!(read(CLAUDE), ChildCheck::Checking);
    assert_eq!(read(&id(MAIN)), ChildCheck::Checked);
    assert!(
        store
            .claude_launch_child_open(CLAUDE, Host::Claude)
            .unwrap()
    );
    let attempt = store.child_check_attempt(CLAUDE).unwrap().unwrap().required;
    store
        .settle_child_checks(&[(CLAUDE, attempt, KEY_A)])
        .unwrap();
    assert_eq!(read(CLAUDE), ChildCheck::Checked);
    // The same launch published again, after an ordinary append of its
    // caller, is not new: nothing moves.
    assert_eq!(publish(&mut store, &[candidate(CLAUDE, "call_a")], 300), 0);
    assert_eq!(
        store.child_check_attempt(CLAUDE).unwrap().unwrap().required,
        attempt
    );
    // Equal structural positions do not prove the launch command and time
    // still match. The changed digest reopens only its named child.
    let mut edited = candidate(CLAUDE, "call_a");
    edited.launch_check_fingerprint = Some("1".repeat(64));
    assert_eq!(publish(&mut store, &[edited], 350), 1);
    assert_eq!(
        store.child_check_attempt(CLAUDE).unwrap().unwrap().required,
        attempt + 1
    );
    assert_eq!(read(&id(MAIN)), ChildCheck::Checked);
    // A changed launch is new evidence again.
    publish(&mut store, &[candidate(CLAUDE, "call_b")], 400);
    assert_eq!(
        store.child_check_attempt(CLAUDE).unwrap().unwrap().required,
        attempt + 2
    );
    assert_eq!(read(&id(MAIN)), ChildCheck::Checked);
}

#[test]
fn a_schema_21_index_upgrades_with_every_session_unchecked_and_its_rows_kept() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(path(&directory)).unwrap();
    for native in [MAIN, CHILD] {
        session(&mut store, native);
        store
            .upsert_records(&id(native), &[record(native, 1)], false)
            .unwrap();
    }
    drop(store);
    // Back to schema 21: the table and columns this version adds are gone.
    {
        let connection = sql(&directory);
        connection
            .execute_batch(
                "ALTER TABLE pull_requests DROP COLUMN manual_failed_at; DROP TABLE session_child_checks;
                 ALTER TABLE claude_launch_groups DROP COLUMN unfinished_starts;
                 ALTER TABLE claude_launch_candidates DROP COLUMN child_check_complete;
                 ALTER TABLE claude_launch_candidates DROP COLUMN launch_check_fingerprint;
                 ALTER TABLE claude_launch_staged_candidates DROP COLUMN launch_check_fingerprint;
                 DELETE FROM schema_version WHERE version>=22;",
            )
            .unwrap();
    }
    let counts = |connection: &Connection| -> (i64, i64) {
        connection
            .query_row(
                "SELECT (SELECT count(*) FROM sessions),(SELECT count(*) FROM records)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    };
    let before = counts(&sql(&directory));
    let store = Store::open(path(&directory)).unwrap();
    assert_eq!(store.schema_version().unwrap(), 23);
    assert_eq!(counts(&sql(&directory)), before);
    let rows: i64 = sql(&directory)
        .query_row(
            "SELECT count(*) FROM session_child_checks WHERE required_generation=1
               AND completed_generation IS NULL AND own_check_key IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 2);
    assert_eq!(check(&directory, MAIN), ChildCheck::Checking);
    // Both added columns exist, every launch row's decision unknown.
    for (table, column) in [
        ("claude_launch_groups", "unfinished_starts"),
        ("claude_launch_candidates", "child_check_complete"),
    ] {
        let found: i64 = sql(&directory)
            .query_row(
                &format!("SELECT count(*) FROM pragma_table_info('{table}') WHERE name='{column}'"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1, "{table}.{column}");
    }
}

/// A launch set aside without a decision keeps naming its child as
/// unchecked until a decision is recorded; a restarted worker reopens it
/// once, and a decided rejection closes it.
#[test]
fn an_undecided_launch_holds_its_child_until_a_decision() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(path(&directory)).unwrap();
    let mut meta = SessionMeta::new(CLAUDE, "claude", SessionSource::Transcript);
    meta.native_session_id = Some(CLAUDE.into());
    store.upsert_session(&meta, false).unwrap();
    store
        .upsert_records(CLAUDE, &[record(CLAUDE, 1)], false)
        .unwrap();
    publish(&mut store, &[candidate(CLAUDE, "call_a")], 100);
    let row = store
        .claude_launch_candidates_for_child(CLAUDE, None, 10)
        .unwrap()
        .remove(0);
    assert!(
        store
            .claude_launch_child_open(CLAUDE, Host::Claude)
            .unwrap()
    );
    assert!(store.defer_claude_launch_child(&row).unwrap());
    assert!(
        store
            .claude_launch_child_open(CLAUDE, Host::Claude)
            .unwrap()
    );
    // Not a timed retry.
    assert!(!store.claude_launch_retry_pending().unwrap());
    assert_eq!(
        store.reopen_undecided_claude_launch_children().unwrap(),
        [CALLER.to_owned()]
    );
    assert!(
        store
            .reopen_undecided_claude_launch_children()
            .unwrap()
            .is_empty()
    );
    let row = store
        .claude_launch_candidates_for_child(CLAUDE, None, 10)
        .unwrap()
        .remove(0);
    assert!(
        store
            .set_claude_launch_child_state(&row, xt_store::claude_launch::ChildState::Rejected)
            .unwrap()
    );
    assert!(
        !store
            .claude_launch_child_open(CLAUDE, Host::Claude)
            .unwrap()
    );
    assert!(
        store
            .reopen_undecided_claude_launch_children()
            .unwrap()
            .is_empty()
    );
}
