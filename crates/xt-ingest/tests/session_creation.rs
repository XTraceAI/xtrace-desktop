//! Codex sub-sessions from the host's own typed spawn record.
//!
//! Every file here is written by the test: short synthetic Codex rollouts
//! whose opening `session_meta` carries a typed `source`, never a real
//! conversation. The suite proves that only a thread's own verified opening
//! header naming a distinct full parent thread relates it; that `guardian`,
//! `exec`, `history_base`, a foreign, moved, aliased, torn or ambiguous source
//! relates nothing; that replay is stable; and that sessions indexed before
//! relations existed are classified by the bootstrap pass without any new
//! event, without writing a host file, and resumably; and that work a bounded
//! pass leaves (a rescan's unchanged threads, a backlog past one chunk, an
//! unfinished bootstrap) is carried on to the end without another event; that
//! a restarted worker recovers what an earlier run left queued from the index
//! alone; and that a background pass announces a stored change once, after
//! it commits, and nothing else.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use xt_fixtures::TempDb;
use xt_ingest::native::{
    HostStatus, ImportRequest, ProducerSource, SessionOutcome, import_native, import_reader_lines,
    readers_cli::{CancelToken, ReaderOutcome, read_pin},
    session_creation::{
        MAX_SPAWN_PROBES, SpawnBacklog, SpawnOutcome, SpawnRefusal, bootstrap_codex_spawns,
        continue_codex_spawns, record_codex_spawns, spawn_limits,
    },
    session_titles::{TitleLimits, Untitled},
    watch::{ProbePoint, TailEvent, Tailer, WatchConfig},
};
use xt_store::{
    Host, Store,
    child_fact::{ChildEvidence, ChildFactDisposition},
    creation::{
        CODEX_THREAD_SPAWN_VERSION, CreationBootstrap, CreationDisposition, CreationEvidence,
        CreationWitness, SessionCreationProof,
    },
    session_list::{ParentEvidence, SessionFilter},
};

const T: i64 = 1_788_782_400_000;
const PARENT: &str = "019a0000-0000-7000-8000-0000000000aa";
const CHILD: &str = "019a0000-0000-7000-8000-0000000000bb";
const OTHER: &str = "019a0000-0000-7000-8000-0000000000cc";
const UNINDEXED: &str = "019a0000-0000-7000-8000-0000000000dd";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn home(root: &Path) -> PathBuf {
    let home = root.join("home");
    fs::create_dir_all(home.join(".codex/sessions/2026/09/07")).unwrap();
    home
}

fn rollout_path(home: &Path, native: &str) -> PathBuf {
    rollout_at(home, "12-00-00", native)
}

fn rollout_at(home: &Path, time: &str, native: &str) -> PathBuf {
    home.join(".codex/sessions/2026/09/07")
        .join(format!("rollout-2026-09-07T{time}-{native}.jsonl"))
}

/// A `session_meta` for `native` with this `source`, and its long
/// instructions, as Codex writes a first-level spawn: its `payload.session_id`
/// is the root of its spawn tree, which is the parent itself here; any other
/// header carries none.
fn header(native: &str, source: Value) -> String {
    let session = source
        .pointer("/subagent/thread_spawn/parent_thread_id")
        .cloned();
    header_with(native, source, session)
}

/// [`header`] with exactly this `payload.session_id`, or none.
fn header_with(native: &str, source: Value, session: Option<Value>) -> String {
    let mut value = json!({"timestamp": "2026-09-07T12:00:00.000Z", "type": "session_meta",
           "payload": {"id": native, "originator": "codex_cli_rs", "cwd": "/repo/fixture",
                       "source": source, "instructions": "long instructions ".repeat(64)}});
    if let Some(session) = session {
        value["payload"]["session_id"] = session;
    }
    value.to_string()
}

/// `line` with this explicit `payload.parent_thread_id`, as current Codex
/// headers carry beside the typed spawn.
fn explicit(line: &str, parent: Value) -> String {
    let mut value: Value = serde_json::from_str(line).unwrap();
    value["payload"]["parent_thread_id"] = parent;
    value.to_string()
}

fn spawned_by(parent: &str) -> Value {
    json!({"subagent": {"thread_spawn": {
        "parent_thread_id": parent, "depth": 1,
        "agent_path": "/root/synthetic_task", "agent_nickname": null, "agent_role": null}}})
}

/// A rollout: `first`, a prompt that is never read, then a response.
fn rollout(first: &str) -> String {
    [
        first.to_owned(),
        json!({"type": "response_item", "payload": {"type": "message", "role": "user",
               "content": [{"type": "input_text", "text": "Synthetic prompt never read"}]}})
        .to_string(),
    ]
    .iter()
    .map(|line| format!("{line}\n"))
    .collect()
}

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

/// Index Codex sessions exactly as a reader stream does: each session's
/// header names `path`, which becomes its recorded locator. No relation is
/// recorded: the stream alone never reads a rollout.
fn index(store: &mut Store, sessions: &[(&str, &Path)]) {
    let mut lines = Vec::new();
    for (native, path) in sessions {
        lines.push(
            json!({"type": "session", "host": "codex", "native_session_id": native,
                   "conversation_id": format!("codex-{native}"), "source_surface": "codex_cli",
                   "started_at": "2026-09-07T12:00:00.000Z", "cwd": "/repo/fixture",
                   "git_branch": null, "title": null, "path": path.display().to_string(),
                   "mtime": 1788782400.0})
            .to_string(),
        );
        lines.push(
            json!({"uuid": format!("5555{}-5555-4555-8555-000000000000", &native[native.len() - 4..]),
                   "type": "user", "cwd": "/repo/fixture", "timestamp": "2026-09-07T12:00:01Z",
                   "message": {"role": "user", "content": [{"type": "text", "text": "turn"}]}})
            .to_string(),
        );
    }
    let report = import_reader_lines(
        store,
        Host::Codex,
        "test".into(),
        lines.into_iter().map(Ok),
        T,
        || {
            Ok(ReaderOutcome {
                diagnostics: vec![],
                complete: true,
            })
        },
    );
    assert_eq!(report.status, HostStatus::Complete, "{report:?}");
}

fn probe(store: &mut Store, home: &Path, natives: &[&str]) -> Vec<SpawnOutcome> {
    record_codex_spawns(store, home, natives, spawn_limits(), None, T)
        .unwrap()
        .probes
        .into_iter()
        .map(|probe| probe.outcome)
        .collect()
}

/// `(child, parent)` for every listed row that shows a parent.
fn shown(store: &Store) -> Vec<(String, String)> {
    store
        .sessions_page_filtered(&SessionFilter::default(), None)
        .unwrap()
        .into_iter()
        .filter_map(|row| Some((row.id, row.parent?.session_id)))
        .collect()
}

fn id(native: &str) -> String {
    format!("codex-{native}")
}

fn refused(why: SpawnRefusal) -> SpawnOutcome {
    SpawnOutcome::Refused(why)
}

fn spawned(parent: &str) -> SpawnOutcome {
    SpawnOutcome::Spawned {
        parent_native_session_id: parent.to_owned(),
    }
}

/// Every file under `home`, with its bytes and modification time.
fn snapshot(home: &Path) -> Vec<(PathBuf, Vec<u8>, std::time::SystemTime)> {
    let mut files = Vec::new();
    let mut stack = vec![home.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                files.push((
                    path.clone(),
                    fs::read(&path).unwrap(),
                    meta.modified().unwrap(),
                ));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn a_typed_spawn_names_its_exact_indexed_parent_and_replays_stably() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let parent = rollout_path(&home, PARENT);
    let child = rollout_path(&home, CHILD);
    let other = rollout_path(&home, OTHER);
    write(&parent, &rollout(&header(PARENT, json!("cli"))));
    write(&child, &rollout(&header(CHILD, spawned_by(PARENT))));
    write(&other, &rollout(&header(OTHER, json!("cli"))));
    let mut db = TempDb::empty().unwrap();
    index(
        db.store_mut(),
        &[(PARENT, &parent), (CHILD, &child), (OTHER, &other)],
    );
    let before = snapshot(&home);
    assert_eq!(
        probe(db.store_mut(), &home, &[PARENT, CHILD, OTHER]),
        [
            SpawnOutcome::NotSpawned,
            spawned(PARENT),
            SpawnOutcome::NotSpawned
        ]
    );
    // The header's parent, never another indexed session.
    assert_eq!(shown(db.store()), [(id(CHILD), id(PARENT))]);
    let row = db
        .store()
        .sessions_page_filtered(&SessionFilter::default(), None)
        .unwrap()
        .into_iter()
        .find(|row| row.id == id(CHILD))
        .unwrap();
    assert_eq!(row.parent.unwrap().evidence, ParentEvidence::NativeSpawn);
    // Replay, also after a reopen, is stable.
    let recording =
        record_codex_spawns(db.store_mut(), &home, &[CHILD], spawn_limits(), None, T).unwrap();
    assert_eq!(
        recording.dispositions,
        [(CHILD.to_owned(), CreationDisposition::AlreadyRecorded)]
    );
    let path = db.path().to_path_buf();
    let mut reopened = Store::open(&path).unwrap();
    let recording =
        record_codex_spawns(&mut reopened, &home, &[CHILD], spawn_limits(), None, T).unwrap();
    assert_eq!(
        recording.dispositions,
        [(CHILD.to_owned(), CreationDisposition::AlreadyRecorded)]
    );
    assert_eq!(shown(&reopened), [(id(CHILD), id(PARENT))]);
    assert_eq!(snapshot(&home), before, "no host file was written");
}

#[test]
fn guardian_exec_history_base_and_role_lookalikes_are_not_spawns() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let parent = rollout_path(&home, PARENT);
    write(&parent, &rollout(&header(PARENT, json!("cli"))));
    let cases: Vec<(&str, String)> = vec![
        // A guardian or review subagent names no parent.
        (
            "019a0000-0000-7000-8000-000000000001",
            header(
                "019a0000-0000-7000-8000-000000000001",
                json!({"subagent": {"other": "guardian"}}),
            ),
        ),
        (
            "019a0000-0000-7000-8000-000000000002",
            header(
                "019a0000-0000-7000-8000-000000000002",
                json!({"subagent": "review"}),
            ),
        ),
        (
            "019a0000-0000-7000-8000-000000000003",
            header("019a0000-0000-7000-8000-000000000003", json!("exec")),
        ),
        // A history continued from the parent's rollout is not a spawn.
        ("019a0000-0000-7000-8000-000000000004", {
            let mut value: Value = serde_json::from_str(&header(
                "019a0000-0000-7000-8000-000000000004",
                json!("cli"),
            ))
            .unwrap();
            value["payload"]["history_base"] = json!({"thread_id": PARENT, "end_byte_offset": 10});
            value.to_string()
        }),
        // A role, task path or nickname outside a typed spawn is not one.
        (
            "019a0000-0000-7000-8000-000000000005",
            header(
                "019a0000-0000-7000-8000-000000000005",
                json!({"agent_path": "/root/task", "agent_role": "reviewer", "parent_thread_id": PARENT}),
            ),
        ),
    ];
    let mut sessions: Vec<(&str, PathBuf)> = vec![(PARENT, parent.clone())];
    for (native, line) in &cases {
        let path = rollout_path(&home, native);
        write(&path, &rollout(line));
        sessions.push((native, path));
    }
    let mut db = TempDb::empty().unwrap();
    let indexed: Vec<(&str, &Path)> = sessions.iter().map(|(n, p)| (*n, p.as_path())).collect();
    index(db.store_mut(), &indexed);
    let natives: Vec<&str> = cases.iter().map(|(native, _)| *native).collect();
    assert!(
        probe(db.store_mut(), &home, &natives)
            .iter()
            .all(|outcome| *outcome == SpawnOutcome::NotSpawned)
    );
    assert!(shown(db.store()).is_empty());
}

#[test]
fn foreign_malformed_torn_or_late_headers_abstain() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let parent = rollout_path(&home, PARENT);
    write(&parent, &rollout(&header(PARENT, json!("cli"))));
    let n = |index: u32| format!("019a0000-0000-7000-8000-{index:012}");
    let mut late = rollout(&json!({"type": "turn_context", "payload": {}}).to_string());
    late.push_str(&format!("{}\n", header(&n(14), spawned_by(PARENT))));
    let cases: Vec<(String, String, SpawnOutcome)> = vec![
        // Another thread's header in this thread's file (a wrong child).
        (
            n(11),
            rollout(&header(OTHER, spawned_by(PARENT))),
            refused(SpawnRefusal::IdentityMismatch),
        ),
        // A spawn whose explicit parent names a third thread.
        (
            n(12),
            rollout(&explicit(
                &header_with(&n(12), spawned_by(PARENT), Some(json!(PARENT))),
                json!(OTHER),
            )),
            refused(SpawnRefusal::ContradictedSpawn),
        ),
        // A torn opening line: no newline ends it.
        (
            n(13),
            header(&n(13), spawned_by(PARENT)),
            refused(SpawnRefusal::Source(Untitled::IdentityMismatch)),
        ),
        // A spawn record that is not the opening line.
        (n(14), late, refused(SpawnRefusal::NoOpeningHeader)),
        // A short, uppercase or missing parent identity.
        (
            n(15),
            rollout(&header(&n(15), spawned_by("019a0000"))),
            refused(SpawnRefusal::MalformedSpawn),
        ),
        (
            n(16),
            rollout(&header(&n(16), spawned_by(&PARENT.to_uppercase()))),
            refused(SpawnRefusal::MalformedSpawn),
        ),
        (
            n(17),
            rollout(&header(
                &n(17),
                json!({"subagent": {"thread_spawn": {"depth": 1}}}),
            )),
            refused(SpawnRefusal::MalformedSpawn),
        ),
        // A spawn beside another subagent kind.
        (
            n(18),
            rollout(&header(
                &n(18),
                json!({"subagent": {"thread_spawn": {"parent_thread_id": PARENT}, "other": "guardian"}}),
            )),
            refused(SpawnRefusal::MalformedSpawn),
        ),
        // A thread naming itself.
        (
            n(19),
            rollout(&header(&n(19), spawned_by(&n(19)))),
            refused(SpawnRefusal::SelfSpawn),
        ),
    ];
    let mut sessions: Vec<(String, PathBuf)> = vec![(PARENT.into(), parent)];
    for (native, body, _) in &cases {
        let path = rollout_path(&home, native);
        write(&path, body);
        sessions.push((native.clone(), path));
    }
    let mut db = TempDb::empty().unwrap();
    let indexed: Vec<(&str, &Path)> = sessions
        .iter()
        .map(|(n, p)| (n.as_str(), p.as_path()))
        .collect();
    index(db.store_mut(), &indexed);
    let natives: Vec<&str> = cases.iter().map(|(native, _, _)| native.as_str()).collect();
    let outcomes = probe(db.store_mut(), &home, &natives);
    for ((native, _, want), got) in cases.iter().zip(&outcomes) {
        assert_eq!(got, want, "{native}");
    }
    // An overlong opening line is never partly parsed.
    let long = n(20);
    let path = rollout_path(&home, &long);
    write(&path, &rollout(&header(&long, spawned_by(PARENT))));
    index(db.store_mut(), &[(&long, &path)]);
    let limits = TitleLimits {
        max_line_bytes: 64,
        ..spawn_limits()
    };
    let recording = record_codex_spawns(db.store_mut(), &home, &[&long], limits, None, T).unwrap();
    assert_eq!(
        recording.probes[0].outcome,
        refused(SpawnRefusal::Source(Untitled::LineTooLong))
    );
    assert!(shown(db.store()).is_empty());
}

/// The spawned-header grammar: `payload.id` is the child,
/// `thread_spawn.parent_thread_id` the immediate parent, `payload.session_id`
/// the root of the spawn tree (the parent itself at the first level, another
/// thread deeper), and, in current headers, `payload.parent_thread_id` the
/// immediate parent again. The root is never compared with the parent: a
/// nested spawn whose root differs is related to its typed parent, with or
/// without the explicit field. A missing, malformed or child-valued root, or
/// an explicit parent that disagrees, relates nothing; a `session_id` alone,
/// beside a guardian, a role or path, or a `history_base`, relates nothing,
/// and neither does a foreign `payload.id`.
#[test]
fn a_spawn_is_related_to_its_typed_parent_whatever_root_it_shares() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let parent = rollout_path(&home, PARENT);
    let other = rollout_path(&home, OTHER);
    write(&parent, &rollout(&header(PARENT, json!("cli"))));
    write(&other, &rollout(&header(OTHER, json!("cli"))));
    let n = |index: u32| format!("019a0000-0000-7000-8000-{index:012}");
    let spawn = spawned_by(PARENT);
    let paginated = |native: &str, session: Value| {
        let mut value: Value =
            serde_json::from_str(&header_with(native, spawned_by(PARENT), Some(session))).unwrap();
        value["payload"]["history_mode"] = json!("paginated");
        value["payload"]["history_base"] = Value::Null;
        value.to_string()
    };
    let with_base = |native: &str, source: Value| {
        let mut value: Value =
            serde_json::from_str(&header_with(native, source, Some(json!(PARENT)))).unwrap();
        value["payload"]["history_base"] = json!({"thread_id": PARENT, "end_byte_offset": 10});
        value.to_string()
    };
    let cases: Vec<(String, String, SpawnOutcome)> = vec![
        // The observed shape, flat and paginated.
        (
            n(41),
            header_with(&n(41), spawn.clone(), Some(json!(PARENT))),
            spawned(PARENT),
        ),
        (n(42), paginated(&n(42), json!(PARENT)), spawned(PARENT)),
        // Current headers: the explicit immediate parent beside a root that
        // is the parent (first level) or another thread (nested), flat or
        // paginated; a null explicit field is a header without it.
        (
            n(55),
            explicit(
                &header_with(&n(55), spawn.clone(), Some(json!(PARENT))),
                json!(PARENT),
            ),
            spawned(PARENT),
        ),
        (
            n(56),
            explicit(
                &header_with(&n(56), spawn.clone(), Some(json!(OTHER))),
                json!(PARENT),
            ),
            spawned(PARENT),
        ),
        (
            n(57),
            explicit(&paginated(&n(57), json!(OTHER)), json!(PARENT)),
            spawned(PARENT),
        ),
        (
            n(58),
            explicit(
                &header_with(&n(58), spawn.clone(), Some(json!(OTHER))),
                Value::Null,
            ),
            spawned(PARENT),
        ),
        // An explicit parent that is another thread, the root, the child,
        // differently spelled or not a string: no parent is read from either
        // field.
        (
            n(59),
            explicit(
                &header_with(&n(59), spawn.clone(), Some(json!(PARENT))),
                json!(OTHER),
            ),
            refused(SpawnRefusal::ContradictedSpawn),
        ),
        (
            n(60),
            explicit(
                &header_with(&n(60), spawn.clone(), Some(json!(OTHER))),
                json!(OTHER),
            ),
            refused(SpawnRefusal::ContradictedSpawn),
        ),
        (
            n(61),
            explicit(
                &header_with(&n(61), spawn.clone(), Some(json!(PARENT))),
                json!(n(61)),
            ),
            refused(SpawnRefusal::ContradictedSpawn),
        ),
        (
            n(62),
            explicit(
                &header_with(&n(62), spawn.clone(), Some(json!(PARENT))),
                json!(PARENT.to_uppercase()),
            ),
            refused(SpawnRefusal::ContradictedSpawn),
        ),
        (
            n(63),
            explicit(
                &header_with(&n(63), spawn.clone(), Some(json!(PARENT))),
                json!(7),
            ),
            refused(SpawnRefusal::ContradictedSpawn),
        ),
        // An agreeing explicit parent cannot stand in for a root that is
        // missing or the child.
        (
            n(64),
            explicit(&header_with(&n(64), spawn.clone(), None), json!(PARENT)),
            refused(SpawnRefusal::UncorroboratedSpawn),
        ),
        (
            n(65),
            explicit(
                &header_with(&n(65), spawn.clone(), Some(json!(n(65)))),
                json!(PARENT),
            ),
            refused(SpawnRefusal::UncorroboratedSpawn),
        ),
        // A missing, null, non-string, child-valued or differently spelled
        // root.
        (
            n(43),
            header_with(&n(43), spawn.clone(), None),
            refused(SpawnRefusal::UncorroboratedSpawn),
        ),
        (
            n(44),
            header_with(&n(44), spawn.clone(), Some(Value::Null)),
            refused(SpawnRefusal::UncorroboratedSpawn),
        ),
        (
            n(45),
            header_with(&n(45), spawn.clone(), Some(json!(7))),
            refused(SpawnRefusal::UncorroboratedSpawn),
        ),
        (
            n(46),
            paginated(&n(46), json!(n(46))),
            refused(SpawnRefusal::UncorroboratedSpawn),
        ),
        (
            n(48),
            header_with(&n(48), spawn.clone(), Some(json!(PARENT.to_uppercase()))),
            refused(SpawnRefusal::UncorroboratedSpawn),
        ),
        // A header written before the explicit field: a nested spawn, whose
        // root is another thread, is held to the rest of the typed contract.
        (
            n(47),
            header_with(&n(47), spawn.clone(), Some(json!(OTHER))),
            spawned(PARENT),
        ),
        // Another thread's `payload.id`, though parent and session_id agree.
        (
            n(49),
            header_with(OTHER, spawn.clone(), Some(json!(PARENT))),
            refused(SpawnRefusal::IdentityMismatch),
        ),
        // A parent-valued session_id with no typed spawn: alone, beside a
        // guardian, a role or task path, or a history continued from the
        // parent's rollout.
        (
            n(50),
            header_with(&n(50), json!("cli"), Some(json!(PARENT))),
            refused(SpawnRefusal::IdentityMismatch),
        ),
        (
            n(51),
            header_with(
                &n(51),
                json!({"subagent": {"other": "guardian"}}),
                Some(json!(PARENT)),
            ),
            SpawnOutcome::Reviewer {
                parent_native_session_id: PARENT.into(),
            },
        ),
        (
            n(52),
            header_with(
                &n(52),
                json!({"agent_path": "/root/task", "agent_role": "reviewer", "parent_thread_id": PARENT}),
                Some(json!(PARENT)),
            ),
            refused(SpawnRefusal::IdentityMismatch),
        ),
        (
            n(53),
            with_base(&n(53), json!("cli")),
            refused(SpawnRefusal::IdentityMismatch),
        ),
        // A person's session whose session_id is its own thread.
        (
            n(54),
            header_with(&n(54), json!("cli"), Some(json!(n(54)))),
            SpawnOutcome::NotSpawned,
        ),
    ];
    let mut sessions: Vec<(String, PathBuf)> = vec![(PARENT.into(), parent), (OTHER.into(), other)];
    for (native, line, _) in &cases {
        let path = rollout_path(&home, native);
        write(&path, &rollout(line));
        sessions.push((native.clone(), path));
    }
    let mut db = TempDb::empty().unwrap();
    let indexed: Vec<(&str, &Path)> = sessions
        .iter()
        .map(|(n, p)| (n.as_str(), p.as_path()))
        .collect();
    index(db.store_mut(), &indexed);
    let natives: Vec<&str> = cases.iter().map(|(native, _, _)| native.as_str()).collect();
    let outcomes = probe(db.store_mut(), &home, &natives);
    for ((native, _, want), got) in cases.iter().zip(&outcomes) {
        assert_eq!(got, want, "{native}");
    }
    let mut related = shown(db.store());
    related.sort();
    // The Guardian reviewer (51) shows its reviewed session as its parent.
    let mut want: Vec<(String, String)> = [41, 42, 47, 51, 55, 56, 57, 58]
        .into_iter()
        .map(|index| (id(&n(index)), id(PARENT)))
        .collect();
    want.sort();
    assert_eq!(related, want);
}

/// That a thread is an agent's child is kept apart from its parent: a
/// verified header typed as a spawn or as a Guardian gives a child fact, and
/// the known-child bit, even when its parent fields are refused, a Guardian
/// names no usable parent or the parent is not indexed. A header that is not
/// the thread's own, a malformed or self-naming spawn, and a person's session
/// give none. The Human input view is not touched by a fact alone.
#[test]
fn typed_child_status_survives_a_refused_or_missing_parent() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let parent = rollout_path(&home, PARENT);
    write(&parent, &rollout(&header(PARENT, json!("cli"))));
    let n = |index: u32| format!("019a0000-0000-7000-8000-{index:012}");
    let guardian = || json!({"subagent": {"other": "guardian"}});
    // (thread, opening line, outcome, child fact, relation or origin shown)
    let cases: Vec<(String, String, SpawnOutcome, Option<ChildEvidence>, bool)> = vec![
        (
            n(91),
            header(&n(91), spawned_by(PARENT)),
            spawned(PARENT),
            Some(ChildEvidence::CodexThreadSpawn),
            true,
        ),
        // A spawn whose parent is not indexed.
        (
            n(92),
            header(&n(92), spawned_by(UNINDEXED)),
            spawned(UNINDEXED),
            Some(ChildEvidence::CodexThreadSpawn),
            false,
        ),
        // Parent fields refused: no root, or an explicit parent that disagrees.
        (
            n(93),
            header_with(&n(93), spawned_by(PARENT), None),
            refused(SpawnRefusal::UncorroboratedSpawn),
            Some(ChildEvidence::CodexThreadSpawn),
            false,
        ),
        (
            n(94),
            explicit(
                &header_with(&n(94), spawned_by(PARENT), Some(json!(PARENT))),
                json!(OTHER),
            ),
            refused(SpawnRefusal::ContradictedSpawn),
            Some(ChildEvidence::CodexThreadSpawn),
            false,
        ),
        // A Guardian, with its reviewed session or with none usable.
        (
            n(95),
            header_with(&n(95), guardian(), Some(json!(PARENT))),
            SpawnOutcome::Reviewer {
                parent_native_session_id: PARENT.into(),
            },
            Some(ChildEvidence::CodexGuardian),
            true,
        ),
        (
            n(96),
            header_with(&n(96), guardian(), None),
            SpawnOutcome::NotSpawned,
            Some(ChildEvidence::CodexGuardian),
            false,
        ),
        // Nothing about the child: another thread's header, a malformed or
        // self-naming spawn, a person's session.
        (
            n(97),
            header(OTHER, spawned_by(PARENT)),
            refused(SpawnRefusal::IdentityMismatch),
            None,
            false,
        ),
        (
            n(98),
            header(&n(98), spawned_by("019a0000")),
            refused(SpawnRefusal::MalformedSpawn),
            None,
            false,
        ),
        (
            n(99),
            header(&n(99), spawned_by(&n(99))),
            refused(SpawnRefusal::SelfSpawn),
            None,
            false,
        ),
        (
            n(100),
            header(&n(100), json!("cli")),
            SpawnOutcome::NotSpawned,
            None,
            false,
        ),
    ];
    let mut sessions: Vec<(String, PathBuf)> = vec![(PARENT.into(), parent)];
    for (native, line, ..) in &cases {
        let path = rollout_path(&home, native);
        write(&path, &rollout(line));
        sessions.push((native.clone(), path));
    }
    let mut db = TempDb::empty().unwrap();
    let indexed: Vec<(&str, &Path)> = sessions
        .iter()
        .map(|(n, p)| (n.as_str(), p.as_path()))
        .collect();
    index(db.store_mut(), &indexed);
    let natives: Vec<&str> = cases.iter().map(|(native, ..)| native.as_str()).collect();
    let recording =
        record_codex_spawns(db.store_mut(), &home, &natives, spawn_limits(), None, T).unwrap();
    let connection = rusqlite::Connection::open(db.path()).unwrap();
    for ((native, _, outcome, child, linked), probe) in cases.iter().zip(&recording.probes) {
        assert_eq!(&probe.outcome, outcome, "{native}");
        assert_eq!(&probe.child, child, "{native}");
        let facts = db.store().child_facts(&id(native)).unwrap();
        assert_eq!(
            facts
                .iter()
                .map(|(fact, accepted)| (fact.evidence_kind, *accepted))
                .collect::<Vec<_>>(),
            child.iter().map(|kind| (*kind, true)).collect::<Vec<_>>(),
            "{native}"
        );
        let context = xt_store::session_list::context(&connection, &[&id(native)])
            .unwrap()
            .remove(0);
        assert_eq!(context.known_child, child.is_some(), "{native}");
        assert_eq!(context.parent.is_some(), *linked, "{native}");
    }
    // A fact alone leaves the Human input view as it was.
    for native in [n(93), n(94), n(96)] {
        assert!(
            db.store()
                .records(&id(&native))
                .unwrap()
                .iter()
                .all(|record| !record.human_excluded),
            "{native}"
        );
    }
    assert_eq!(
        recording
            .child_facts
            .iter()
            .filter(|(_, disposition)| *disposition == ChildFactDisposition::Recorded)
            .count(),
        6
    );
    // A replay records nothing new.
    let again =
        record_codex_spawns(db.store_mut(), &home, &natives, spawn_limits(), None, T + 1).unwrap();
    assert_eq!(again.changed, 0);
    assert!(
        again
            .child_facts
            .iter()
            .all(|(_, disposition)| *disposition == ChildFactDisposition::AlreadyRecorded)
    );
}

#[test]
fn moved_aliased_outside_or_ambiguous_sources_abstain() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let parent = rollout_path(&home, PARENT);
    write(&parent, &rollout(&header(PARENT, json!("cli"))));
    let body = |native: &str| rollout(&header(native, spawned_by(PARENT)));
    let n = |index: u32| format!("019a0000-0000-7000-8000-{index:012}");
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &[(PARENT, &parent)]);

    // Moved after indexing: the file is elsewhere, never found by name.
    let moved = rollout_path(&home, &n(21));
    write(&moved, &body(&n(21)));
    index(db.store_mut(), &[(&n(21), &moved)]);
    fs::rename(&moved, home.join(".codex").join("moved.jsonl")).unwrap();

    // The recorded name is an alias to a real rollout.
    let aliased = rollout_path(&home, &n(22));
    let real = temp.path().join("elsewhere.jsonl");
    write(&real, &body(&n(22)));
    std::os::unix::fs::symlink(&real, &aliased).unwrap();
    index(db.store_mut(), &[(&n(22), &aliased)]);

    // A rollout recorded outside the Codex history root.
    let outside = temp
        .path()
        .join(format!("rollout-2026-09-07T12-00-00-{}.jsonl", n(23)));
    write(&outside, &body(&n(23)));
    index(db.store_mut(), &[(&n(23), &outside)]);

    // Two distinct root rollouts recorded for one thread.
    let first = rollout_at(&home, "12-00-00", &n(24));
    let second = rollout_at(&home, "13-00-00", &n(24));
    write(&first, &body(&n(24)));
    write(&second, &body(&n(24)));
    index(db.store_mut(), &[(&n(24), &first)]);
    index(db.store_mut(), &[(&n(24), &second)]);

    // A directory where the rollout should be.
    let directory = rollout_path(&home, &n(25));
    fs::create_dir_all(&directory).unwrap();
    index(db.store_mut(), &[(&n(25), &directory)]);

    let ids = [n(21), n(22), n(23), n(24), n(25)];
    let natives: Vec<&str> = ids.iter().map(String::as_str).collect();
    assert_eq!(
        probe(db.store_mut(), &home, &natives),
        [
            refused(SpawnRefusal::Source(Untitled::Missing)),
            refused(SpawnRefusal::Source(Untitled::Outside)),
            refused(SpawnRefusal::Source(Untitled::Outside)),
            refused(SpawnRefusal::AmbiguousRoot),
            refused(SpawnRefusal::Source(Untitled::Outside)),
        ]
    );
    // An identifier that is not a full lowercase thread, or not indexed.
    assert_eq!(
        probe(
            db.store_mut(),
            &home,
            &["../x", &CHILD.to_uppercase(), UNINDEXED]
        ),
        [
            refused(SpawnRefusal::InvalidIdentifier),
            refused(SpawnRefusal::InvalidIdentifier),
            refused(SpawnRefusal::NotIndexed),
        ]
    );
    assert!(shown(db.store()).is_empty());
}

#[test]
fn a_rewritten_header_naming_another_parent_is_withheld_not_moved() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let parent = rollout_path(&home, PARENT);
    let other = rollout_path(&home, OTHER);
    let child = rollout_path(&home, CHILD);
    write(&parent, &rollout(&header(PARENT, json!("cli"))));
    write(&other, &rollout(&header(OTHER, json!("cli"))));
    write(&child, &rollout(&header(CHILD, spawned_by(PARENT))));
    let mut db = TempDb::empty().unwrap();
    index(
        db.store_mut(),
        &[(PARENT, &parent), (OTHER, &other), (CHILD, &child)],
    );
    probe(db.store_mut(), &home, &[CHILD]);
    assert_eq!(shown(db.store()), [(id(CHILD), id(PARENT))]);
    // Another typed parent in a header with no root, the child as root, or
    // an explicit parent that disagrees is malformed evidence, not a second
    // claim: the accepted relation is untouched.
    for (line, why) in [
        (
            header_with(CHILD, spawned_by(OTHER), None),
            SpawnRefusal::UncorroboratedSpawn,
        ),
        (
            header_with(CHILD, spawned_by(OTHER), Some(json!(CHILD))),
            SpawnRefusal::UncorroboratedSpawn,
        ),
        (
            explicit(
                &header_with(CHILD, spawned_by(OTHER), Some(json!(PARENT))),
                json!(PARENT),
            ),
            SpawnRefusal::ContradictedSpawn,
        ),
    ] {
        write(&child, &rollout(&line));
        let recording =
            record_codex_spawns(db.store_mut(), &home, &[CHILD], spawn_limits(), None, T).unwrap();
        assert_eq!(recording.probes[0].outcome, refused(why));
        assert!(recording.dispositions.is_empty());
        assert_eq!(recording.changed, 0);
        assert_eq!(shown(db.store()), [(id(CHILD), id(PARENT))]);
    }
    // A consistent one is.
    write(&child, &rollout(&header(CHILD, spawned_by(OTHER))));
    let recording =
        record_codex_spawns(db.store_mut(), &home, &[CHILD], spawn_limits(), None, T).unwrap();
    assert_eq!(
        recording.dispositions,
        [(CHILD.to_owned(), CreationDisposition::Conflicted)]
    );
    assert!(shown(db.store()).is_empty());
    let (stored, conflicted) = db.store().session_creation(&id(CHILD)).unwrap().unwrap();
    assert_eq!(stored.parent_native_session_id, PARENT);
    assert!(conflicted);
}

#[test]
fn a_mutual_spawn_records_one_direction_only() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let parent = rollout_path(&home, PARENT);
    let child = rollout_path(&home, CHILD);
    write(&parent, &rollout(&header(PARENT, spawned_by(CHILD))));
    write(&child, &rollout(&header(CHILD, spawned_by(PARENT))));
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &[(PARENT, &parent), (CHILD, &child)]);
    let recording = record_codex_spawns(
        db.store_mut(),
        &home,
        &[CHILD, PARENT],
        spawn_limits(),
        None,
        T,
    )
    .unwrap();
    assert_eq!(
        recording.dispositions,
        [
            (CHILD.to_owned(), CreationDisposition::Recorded),
            (
                PARENT.to_owned(),
                CreationDisposition::Abstained(xt_store::creation::CreationAbstention::Cycle)
            )
        ]
    );
    assert_eq!(shown(db.store()), [(id(CHILD), id(PARENT))]);
}

/// The acceptance case for sessions indexed before relations existed: no new
/// session, record or source event, only the pass over what the index holds.
#[test]
fn the_bootstrap_classifies_an_existing_index_resumably_without_new_events() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let n = |index: u32| format!("019a0000-0000-7000-8000-{index:012}");
    let parent = rollout_path(&home, PARENT);
    write(&parent, &rollout(&header(PARENT, json!("cli"))));
    let mut sessions = vec![(PARENT.to_owned(), parent)];
    // Five spawned children, one guardian, and a parent never indexed.
    for index in 31..36 {
        let path = rollout_path(&home, &n(index));
        write(&path, &rollout(&header(&n(index), spawned_by(PARENT))));
        sessions.push((n(index), path));
    }
    let guardian = rollout_path(&home, &n(36));
    write(
        &guardian,
        &rollout(&header(&n(36), json!({"subagent": {"other": "guardian"}}))),
    );
    sessions.push((n(36), guardian));
    let orphan = rollout_path(&home, &n(37));
    write(&orphan, &rollout(&header(&n(37), spawned_by(UNINDEXED))));
    sessions.push((n(37), orphan));
    let mut db = TempDb::empty().unwrap();
    let indexed: Vec<(&str, &Path)> = sessions
        .iter()
        .map(|(n, p)| (n.as_str(), p.as_path()))
        .collect();
    index(db.store_mut(), &indexed);
    let counts = db.store().counts().unwrap();
    let before = snapshot(&home);
    assert!(
        shown(db.store()).is_empty(),
        "indexing alone relates nothing"
    );

    // A cancelled pass decides nothing and saves no progress.
    let cancel = CancelToken::new();
    cancel.cancel();
    let step =
        bootstrap_codex_spawns(db.store_mut(), &home, spawn_limits(), Some(&cancel), T).unwrap();
    assert!(!step.complete);
    assert_eq!(step.spawned, 0);
    let kind = CreationEvidence::CodexThreadSpawn;
    assert_eq!(
        db.store()
            .session_creation_bootstrap(kind, CODEX_THREAD_SPAWN_VERSION)
            .unwrap(),
        CreationBootstrap::default()
    );

    // A budget that holds about two opening lines stops part-way, saving
    // progress up to the first thread it could not read.
    let line = rollout(&header(PARENT, json!("cli")))
        .lines()
        .next()
        .unwrap()
        .len() as u64;
    let limits = TitleLimits {
        max_batch_bytes: 2 * (line + 256),
        ..spawn_limits()
    };
    let step = bootstrap_codex_spawns(db.store_mut(), &home, limits, None, T).unwrap();
    assert!(!step.complete);
    assert!(step.probed >= 1 && step.probed < sessions.len(), "{step:?}");
    let saved = db
        .store()
        .session_creation_bootstrap(kind, CODEX_THREAD_SPAWN_VERSION)
        .unwrap();
    assert!(saved.after_locator.is_some() && !saved.complete);

    // Resumed with the default bounds, the pass finishes.
    let mut resumed = 0;
    loop {
        let step = bootstrap_codex_spawns(db.store_mut(), &home, limits, None, T).unwrap();
        resumed += 1;
        if step.complete {
            break;
        }
        assert!(resumed < 20, "the pass must make progress");
    }
    let mut want: Vec<(String, String)> =
        (31..36).map(|index| (id(&n(index)), id(PARENT))).collect();
    want.sort();
    let mut got = shown(db.store());
    got.sort();
    assert_eq!(got, want);
    // The orphan's relation is kept, not shown, until its parent is indexed.
    assert!(db.store().session_creation(&id(&n(37))).unwrap().is_some());
    assert!(db.store().session_creation(&id(&n(36))).unwrap().is_none());
    // Nothing new was indexed and no host file changed.
    assert_eq!(db.store().counts().unwrap(), counts);
    assert_eq!(snapshot(&home), before);
    // A finished pass reads nothing more.
    let step = bootstrap_codex_spawns(db.store_mut(), &home, spawn_limits(), None, T).unwrap();
    assert!(step.complete);
    assert_eq!((step.probed, step.bytes_read), (0, 0));
}

/// A scan whose reader cannot even start still runs the pass over what the
/// index already holds, so an existing index is classified on the first
/// scan after upgrade.
#[test]
fn a_scan_runs_the_bootstrap_even_when_the_reader_is_unavailable() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let parent = rollout_path(&home, PARENT);
    let child = rollout_path(&home, CHILD);
    write(&parent, &rollout(&header(PARENT, json!("cli"))));
    write(&child, &rollout(&header(CHILD, spawned_by(PARENT))));
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &[(PARENT, &parent), (CHILD, &child)]);
    let missing = temp.path().join("nonexistent-python");
    let report = import_native(
        db.store_mut(),
        &ImportRequest {
            home: &home,
            hosts: &[Host::Codex],
            producer: &ProducerSource::Bundle {
                pin: read_pin(&repo().join(".plugin-pin")).unwrap(),
                root: repo().join("vendor/agent-plugins"),
            },
            python: Some(missing.as_os_str()),
            observed_at: T,
            cancel: None,
        },
    );
    assert_eq!(report.hosts[0].status, HostStatus::MissingRuntime);
    assert_eq!(shown(db.store()), [(id(CHILD), id(PARENT))]);
    assert!(
        db.store()
            .session_creation_bootstrap(
                CreationEvidence::CodexThreadSpawn,
                CODEX_THREAD_SPAWN_VERSION
            )
            .unwrap()
            .complete
    );
}

/// An index that version 1 finished with: its pass complete, a first-level
/// child related and a nested child it refused, whose root `session_id` is
/// not its parent. The version 2 pass reads every indexed root again once
/// and relates the nested child to its typed immediate parent; the stored
/// version 1 relation replays unchanged, the Guardian reviewer is recognized
/// as before, no host file changes, and nothing is changed again by a second
/// pass or a replay.
#[test]
fn the_version_two_pass_reaches_a_nested_child_version_one_refused() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let n = |index: u32| format!("019a0000-0000-7000-8000-{index:012}");
    let (root, middle, nested, guardian) = (PARENT, CHILD, n(81), n(82));
    let paths = [root, middle, nested.as_str(), guardian.as_str()]
        .map(|native| rollout_path(&home, native));
    write(&paths[0], &rollout(&header(root, json!("cli"))));
    write(&paths[1], &rollout(&header(middle, spawned_by(root))));
    write(
        &paths[2],
        &rollout(&explicit(
            &header_with(&nested, spawned_by(middle), Some(json!(root))),
            json!(middle),
        )),
    );
    write(
        &paths[3],
        &rollout(&explicit(
            &header_with(
                &guardian,
                json!({"subagent": {"other": "guardian"}}),
                Some(json!(root)),
            ),
            json!(middle),
        )),
    );
    let mut db = TempDb::empty().unwrap();
    index(
        db.store_mut(),
        &[
            (root, &paths[0]),
            (middle, &paths[1]),
            (&nested, &paths[2]),
            (&guardian, &paths[3]),
        ],
    );
    let kind = CreationEvidence::CodexThreadSpawn;
    db.store_mut()
        .advance_session_creation_bootstrap(
            kind,
            1,
            &CreationBootstrap {
                after_locator: None,
                complete: true,
            },
        )
        .unwrap();
    db.store_mut()
        .record_session_creations(
            &[SessionCreationProof {
                child_session_id: id(middle),
                child_host: Host::Codex,
                child_native_session_id: middle.into(),
                parent_host: Host::Codex,
                parent_native_session_id: root.into(),
                evidence_kind: kind,
                evidence_version: 1,
                witness: CreationWitness::RolloutOpeningSessionMeta,
            }],
            T,
        )
        .unwrap();
    let stored = db.store().session_creation(&id(middle)).unwrap().unwrap();
    assert_eq!(shown(db.store()), [(id(middle), id(root))]);
    assert_eq!(CODEX_THREAD_SPAWN_VERSION, 2);
    assert!(
        !db.store()
            .session_creation_bootstrap(kind, CODEX_THREAD_SPAWN_VERSION)
            .unwrap()
            .complete
    );
    let counts = db.store().counts().unwrap();
    let before = snapshot(&home);

    let step = bootstrap_codex_spawns(db.store_mut(), &home, spawn_limits(), None, T + 1).unwrap();
    assert!(step.complete);
    let mut got = shown(db.store());
    got.sort();
    // The Guardian reviewer shows the session its `session_id` names.
    let mut want = vec![
        (id(middle), id(root)),
        (id(&nested), id(middle)),
        (id(&guardian), id(root)),
    ];
    want.sort();
    assert_eq!(got, want);
    // The version 1 row is exactly as stored; the new one is version 2.
    assert_eq!(
        db.store().session_creation(&id(middle)).unwrap().unwrap(),
        stored
    );
    let (proof, conflicted) = db.store().session_creation(&id(&nested)).unwrap().unwrap();
    assert_eq!(
        (
            proof.parent_native_session_id.as_str(),
            proof.evidence_version,
            conflicted
        ),
        (middle, 2, false)
    );
    // The Guardian reviewer is read as before: its parent still comes from
    // `session_id`, whatever its explicit parent says.
    let reviewers: Vec<String> = db
        .store()
        .sessions_page_filtered(&SessionFilter::default(), None)
        .unwrap()
        .into_iter()
        .filter(|row| row.automated_review)
        .map(|row| row.id)
        .collect();
    assert_eq!(reviewers, [id(&guardian)]);
    assert_eq!(db.store().counts().unwrap(), counts);
    assert_eq!(snapshot(&home), before);

    // A finished pass reads nothing; a replay changes nothing.
    let step = bootstrap_codex_spawns(db.store_mut(), &home, spawn_limits(), None, T + 2).unwrap();
    assert!(step.complete);
    assert_eq!((step.probed, step.bytes_read), (0, 0));
    let natives = [root, middle, nested.as_str(), guardian.as_str()];
    let recording =
        record_codex_spawns(db.store_mut(), &home, &natives, spawn_limits(), None, T + 3).unwrap();
    assert_eq!(
        recording
            .probes
            .iter()
            .map(|probe| probe.outcome.clone())
            .collect::<Vec<_>>(),
        [
            SpawnOutcome::NotSpawned,
            spawned(root),
            spawned(middle),
            SpawnOutcome::Reviewer {
                parent_native_session_id: root.into()
            },
        ]
    );
    assert_eq!(
        recording.dispositions,
        [
            (middle.to_owned(), CreationDisposition::AlreadyRecorded),
            (nested.clone(), CreationDisposition::AlreadyRecorded),
        ]
    );
    assert_eq!(recording.changed, 0);
    assert_eq!(
        db.store().session_creation(&id(middle)).unwrap().unwrap(),
        stored
    );
}

/// Native ingestion of a new thread relates it through the same guarded
/// path, with the bootstrap already finished: the pinned reader indexes the
/// thread, and the scan probes the threads it changed.
#[test]
fn a_scanned_new_thread_is_related_by_the_ingestion_path() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let mut db = TempDb::empty().unwrap();
    db.store_mut()
        .advance_session_creation_bootstrap(
            CreationEvidence::CodexThreadSpawn,
            CODEX_THREAD_SPAWN_VERSION,
            &CreationBootstrap {
                after_locator: None,
                complete: true,
            },
        )
        .unwrap();
    let parent = rollout_path(&home, PARENT);
    let child = rollout_path(&home, CHILD);
    let turn = |native: &str| {
        let mut lines = vec![header(
            native,
            if native == CHILD {
                spawned_by(PARENT)
            } else {
                json!("cli")
            },
        )];
        lines.push(
            json!({"timestamp": "2026-09-07T12:00:01.000Z", "type": "response_item",
                   "payload": {"type": "message", "role": "user",
                               "content": [{"type": "input_text", "text": "Synthetic request"}]}})
            .to_string(),
        );
        lines.push(
            json!({"timestamp": "2026-09-07T12:00:02.000Z", "type": "response_item",
                   "payload": {"type": "message", "role": "assistant",
                               "content": [{"type": "output_text", "text": "Synthetic reply"}]}})
            .to_string(),
        );
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>()
    };
    write(&parent, &turn(PARENT));
    write(&child, &turn(CHILD));
    let report = import_native(
        db.store_mut(),
        &ImportRequest {
            home: &home,
            hosts: &[Host::Codex],
            producer: &ProducerSource::Bundle {
                pin: read_pin(&repo().join(".plugin-pin")).unwrap(),
                root: repo().join("vendor/agent-plugins"),
            },
            python: None,
            observed_at: T,
            cancel: None,
        },
    );
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
    assert!(
        report.hosts[0]
            .sessions
            .iter()
            .any(|session| session.native_session_id.as_deref() == Some(CHILD)),
        "{report:?}"
    );
    assert_eq!(shown(db.store()), [(id(CHILD), id(PARENT))]);
}

fn bundle() -> ProducerSource {
    ProducerSource::Bundle {
        pin: read_pin(&repo().join(".plugin-pin")).unwrap(),
        root: repo().join("vendor/agent-plugins"),
    }
}

/// A rollout the pinned reader indexes: `first`, then one exchange.
fn exchange(first: &str) -> String {
    [
        first.to_owned(),
        json!({"timestamp": "2026-09-07T12:00:01.000Z", "type": "response_item",
               "payload": {"type": "message", "role": "user",
                           "content": [{"type": "input_text", "text": "Synthetic request"}]}})
        .to_string(),
        json!({"timestamp": "2026-09-07T12:00:02.000Z", "type": "response_item",
               "payload": {"type": "message", "role": "assistant",
                           "content": [{"type": "output_text", "text": "Synthetic reply"}]}})
        .to_string(),
    ]
    .iter()
    .map(|line| format!("{line}\n"))
    .collect()
}

/// One ordinary scan through the pinned reader; the child's own outcome.
fn rescan(store: &mut Store, home: &Path) -> SessionOutcome {
    let report = import_native(
        store,
        &ImportRequest {
            home,
            hosts: &[Host::Codex],
            producer: &bundle(),
            python: None,
            observed_at: T,
            cancel: None,
        },
    );
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
    report.hosts[0]
        .sessions
        .iter()
        .find(|session| session.native_session_id.as_deref() == Some(CHILD))
        .expect("the reader reports the child")
        .outcome
        .clone()
}

fn records_new(outcome: &SessionOutcome) -> usize {
    match outcome {
        SessionOutcome::Imported { records_new, .. }
        | SessionOutcome::Partial { records_new, .. } => *records_new,
        SessionOutcome::Skipped { reason } => panic!("skipped: {reason}"),
    }
}

/// The ordinary import path reads an indexed thread's header whether or not
/// the scan added records to it. A header that gains a typed spawn relates
/// the thread; one that loses it keeps the accepted relation (a plain header
/// is no claim of another parent), and so does one naming a different typed
/// parent that its own explicit `parent_thread_id` contradicts; one
/// consistently naming a different parent conflicts the relation and
/// withholds it. Each rescan inserts no record.
#[test]
fn a_rescan_that_adds_no_records_still_reads_a_new_or_changed_header() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let mut db = TempDb::empty().unwrap();
    db.store_mut()
        .advance_session_creation_bootstrap(
            CreationEvidence::CodexThreadSpawn,
            CODEX_THREAD_SPAWN_VERSION,
            &CreationBootstrap {
                after_locator: None,
                complete: true,
            },
        )
        .unwrap();
    let parent = rollout_path(&home, PARENT);
    let other = rollout_path(&home, OTHER);
    let child = rollout_path(&home, CHILD);
    write(&parent, &exchange(&header(PARENT, json!("cli"))));
    write(&other, &exchange(&header(OTHER, json!("cli"))));
    write(&child, &exchange(&header(CHILD, json!("cli"))));
    assert!(records_new(&rescan(db.store_mut(), &home)) > 0);
    assert!(shown(db.store()).is_empty());

    // The same records, now opened by a typed spawn.
    write(&child, &exchange(&header(CHILD, spawned_by(PARENT))));
    assert_eq!(records_new(&rescan(db.store_mut(), &home)), 0);
    assert_eq!(shown(db.store()), [(id(CHILD), id(PARENT))]);

    // A header that names no parent claims no other one.
    write(
        &child,
        &exchange(&header(CHILD, json!({"subagent": {"other": "guardian"}}))),
    );
    assert_eq!(records_new(&rescan(db.store_mut(), &home)), 0);
    assert_eq!(shown(db.store()), [(id(CHILD), id(PARENT))]);

    // A different typed parent whose explicit parent still names the first
    // is contradicted: the accepted relation stays.
    write(
        &child,
        &exchange(&explicit(
            &header_with(CHILD, spawned_by(OTHER), Some(json!(PARENT))),
            json!(PARENT),
        )),
    );
    assert_eq!(records_new(&rescan(db.store_mut(), &home)), 0);
    assert_eq!(shown(db.store()), [(id(CHILD), id(PARENT))]);
    assert!(!db.store().session_creation(&id(CHILD)).unwrap().unwrap().1);

    // A different typed parent, consistent, is a second claim: both are
    // withheld.
    write(&child, &exchange(&header(CHILD, spawned_by(OTHER))));
    assert_eq!(records_new(&rescan(db.store_mut(), &home)), 0);
    assert!(shown(db.store()).is_empty());
    let (stored, conflicted) = db.store().session_creation(&id(CHILD)).unwrap().unwrap();
    assert_eq!(stored.parent_native_session_id, PARENT);
    assert!(conflicted);
}

/// More threads than one probe call takes: a direct call is refused whole,
/// never truncated, and a backlog is read in chunks over as many bounded
/// passes as it needs, reaching a child queued after the first thousand
/// without any new source event.
#[test]
fn a_backlog_past_one_chunk_reaches_a_late_child_over_bounded_passes() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let n = |index: usize| format!("019a0000-0000-7000-8000-{index:012}");
    let total = MAX_SPAWN_PROBES + 50;
    let late = MAX_SPAWN_PROBES + 40;
    let parent = rollout_path(&home, PARENT);
    write(&parent, &rollout(&header(PARENT, json!("cli"))));
    let mut sessions = vec![(PARENT.to_owned(), parent)];
    for index in 0..total {
        let path = rollout_path(&home, &n(index));
        let source = if index == late {
            spawned_by(PARENT)
        } else {
            json!("cli")
        };
        write(&path, &rollout(&header(&n(index), source)));
        sessions.push((n(index), path));
    }
    let mut db = TempDb::empty().unwrap();
    let indexed: Vec<(&str, &Path)> = sessions
        .iter()
        .map(|(n, p)| (n.as_str(), p.as_path()))
        .collect();
    index(db.store_mut(), &indexed);
    db.store_mut()
        .advance_session_creation_bootstrap(
            CreationEvidence::CodexThreadSpawn,
            CODEX_THREAD_SPAWN_VERSION,
            &CreationBootstrap {
                after_locator: None,
                complete: true,
            },
        )
        .unwrap();
    let ids: Vec<String> = (0..total).map(n).collect();
    let natives: Vec<&str> = ids.iter().map(String::as_str).collect();
    assert!(record_codex_spawns(db.store_mut(), &home, &natives, spawn_limits(), None, T).is_err());
    assert!(shown(db.store()).is_empty());

    // Each pass holds about 300 opening lines.
    let line = rollout(&header(PARENT, json!("cli")))
        .lines()
        .next()
        .unwrap()
        .len() as u64;
    let limits = TitleLimits {
        max_batch_bytes: 300 * (line + 256),
        ..spawn_limits()
    };
    let mut backlog = SpawnBacklog::default();
    backlog.add(natives.iter().copied());
    let mut passes = 0;
    while backlog.pending() {
        let progress =
            continue_codex_spawns(db.store_mut(), &home, &mut backlog, limits, None, T).unwrap();
        assert!(progress.advanced(), "pass {passes} moved nothing");
        passes += 1;
        assert!(passes < 20, "the backlog must drain");
    }
    assert!(passes > total / 300, "{passes} passes");
    assert_eq!(backlog.threads(), 0);
    assert_eq!(shown(db.store()), [(id(&n(late)), id(PARENT))]);

    // Unbounded but for the chunk size, one pass reads every chunk.
    let mut backlog = SpawnBacklog::default();
    backlog.add(natives.iter().copied());
    let progress =
        continue_codex_spawns(db.store_mut(), &home, &mut backlog, spawn_limits(), None, T)
            .unwrap();
    assert_eq!(progress.decided, total);
    // What is left is the display checks of the sessions it indexed (and the
    // header of any still checking that this backlog has not read), and then
    // nothing.
    for _ in 0..10 {
        if !backlog.pending() {
            break;
        }
        continue_codex_spawns(db.store_mut(), &home, &mut backlog, spawn_limits(), None, T)
            .unwrap();
    }
    assert!(!backlog.pending());
}

/// The acceptance case for an existing index on the ordinary worker path:
/// the reader cannot run, no source changes, and each pass is cut short by a
/// deliberately small budget and deadline. The worker keeps scheduling
/// passes on its own until the bootstrap reaches the last child.
#[test]
fn the_worker_finishes_an_interrupted_bootstrap_without_a_source_event() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let n = |index: u32| format!("019a0000-0000-7000-8000-{index:012}");
    let parent = rollout_path(&home, PARENT);
    write(&parent, &rollout(&header(PARENT, json!("cli"))));
    let mut sessions = vec![(PARENT.to_owned(), parent)];
    for index in 40..52 {
        let path = rollout_path(&home, &n(index));
        write(&path, &rollout(&header(&n(index), spawned_by(PARENT))));
        sessions.push((n(index), path));
    }
    let database = temp.path().join("index.sqlite");
    {
        let mut store = Store::open(&database).unwrap();
        let indexed: Vec<(&str, &Path)> = sessions
            .iter()
            .map(|(n, p)| (n.as_str(), p.as_path()))
            .collect();
        index(&mut store, &indexed);
    }
    let before = snapshot(&home);
    let line = rollout(&header(PARENT, json!("cli")))
        .lines()
        .next()
        .unwrap()
        .len() as u64;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let events = Arc::clone(&events);
        Box::new(move |event: TailEvent| events.lock().unwrap().push(event))
    };
    let missing = temp.path().join("nonexistent-python");
    let tailer = Tailer::start(
        Store::open(&database).unwrap(),
        WatchConfig {
            home: home.clone(),
            hosts: vec![Host::Codex],
            producer: bundle(),
            python: Some(missing.into_os_string()),
            debounce: Duration::from_millis(200),
            spawn_limits: TitleLimits {
                max_batch_bytes: 2 * (line + 256),
                deadline: Duration::from_millis(250),
                ..spawn_limits()
            },
            probe: None,
        },
        sink,
    );
    assert!(tailer.wait_ready(Duration::from_secs(30)).is_some());
    let reader = Store::open(&database).unwrap();
    let last = id(&n(51));
    let related = |store: &Store| {
        shown(store)
            .into_iter()
            .filter(|(_, parent)| *parent == id(PARENT))
            .count()
    };
    // The initial scan's pass reads about two headers: the last child waits.
    assert!(related(&reader) < 12);
    assert!(reader.session_creation(&last).unwrap().is_none());
    // The pass that relates the last child may stop before the one that
    // finds no more locators and marks the bootstrap finished.
    let finished = |store: &Store| {
        store
            .session_creation_bootstrap(
                CreationEvidence::CodexThreadSpawn,
                CODEX_THREAD_SPAWN_VERSION,
            )
            .unwrap()
            .complete
    };
    let started = Instant::now();
    while reader.session_creation(&last).unwrap().is_none() || !finished(&reader) {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the worker stopped short: {} related",
            related(&reader)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(related(&reader), 12);
    let status = tailer.status();
    tailer.stop();
    // Only the initial scan reconciled: no source event drove the passes,
    // and no host file changed.
    assert_eq!(status.reconciles, 1, "{status:?}");
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .all(|event| !matches!(event, TailEvent::Reconciled { .. }))
    );
    assert_eq!(snapshot(&home), before);
}

/// What a test worker's sink and probe collected.
type Heard<T> = Arc<Mutex<Vec<T>>>;

/// One worker over `database`: its events, and each background pass's
/// `pending` as the pass ends.
fn worker(
    database: &Path,
    home: &Path,
    python: Option<std::ffi::OsString>,
    spawn_limits: TitleLimits,
) -> (Tailer, Heard<TailEvent>, Heard<bool>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let passes = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let events = Arc::clone(&events);
        Box::new(move |event: TailEvent| events.lock().unwrap().push(event))
    };
    let probe = {
        let passes = Arc::clone(&passes);
        Arc::new(move |point: ProbePoint<'_>| {
            if let ProbePoint::SpawnsContinued { pending } = point {
                passes.lock().unwrap().push(pending);
            }
        })
    };
    let tailer = Tailer::start(
        Store::open(database).unwrap(),
        WatchConfig {
            home: home.to_path_buf(),
            hosts: vec![Host::Codex],
            producer: bundle(),
            python,
            debounce: Duration::from_millis(50),
            spawn_limits,
            probe: Some(probe),
        },
        sink,
    );
    assert!(tailer.wait_ready(Duration::from_secs(60)).is_some());
    (tailer, events, passes)
}

/// Wait until a background pass ends with `pending` as given.
fn wait_pass(passes: &Mutex<Vec<bool>>, pending: bool) {
    let started = Instant::now();
    while !passes.lock().unwrap().contains(&pending) {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "no pass ended with pending={pending}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Every change announcement a worker sent.
fn changes(events: &Mutex<Vec<TailEvent>>) -> Vec<usize> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            TailEvent::SessionCreationsChanged { changed } => Some(*changed),
            _ => None,
        })
        .collect()
}

/// A thread the reader indexes after the bootstrap finished waits only in
/// the worker's memory. The worker is shut down before any pass can read it,
/// then restarted with the reader unavailable and no source change: the
/// sweep over the index's own root locators relates it, and the pass that
/// committed says so once, after the commit. A restart that finds nothing new
/// is silent, and so is a replay of a relation already withheld; a first
/// conflict found the same way is announced once.
#[test]
fn a_restart_recovers_an_unread_thread_from_the_index_and_announces_each_change_once() {
    const GUARDIAN: &str = "019a0000-0000-7000-8000-0000000000ee";
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let database = temp.path().join("index.sqlite");
    Store::open(&database)
        .unwrap()
        .advance_session_creation_bootstrap(
            CreationEvidence::CodexThreadSpawn,
            CODEX_THREAD_SPAWN_VERSION,
            &CreationBootstrap {
                after_locator: None,
                complete: true,
            },
        )
        .unwrap();
    let child = rollout_path(&home, CHILD);
    write(
        &rollout_path(&home, PARENT),
        &exchange(&header(PARENT, json!("cli"))),
    );
    write(
        &rollout_path(&home, OTHER),
        &exchange(&header(OTHER, json!("cli"))),
    );
    write(
        &rollout_path(&home, GUARDIAN),
        &exchange(&header(
            GUARDIAN,
            json!({"subagent": {"other": "guardian"}}),
        )),
    );
    write(&child, &exchange(&header(CHILD, spawned_by(PARENT))));
    let missing = || Some(temp.path().join("nonexistent-python").into_os_string());
    let reader = Store::open(&database).unwrap();

    // The pinned reader indexes every thread, which queues them; no pass may
    // read a byte, so the child stays queued until the worker is shut down.
    let (tailer, events, passes) = worker(
        &database,
        &home,
        None,
        TitleLimits {
            max_batch_bytes: 0,
            ..spawn_limits()
        },
    );
    let listed: Vec<String> = reader
        .sessions_page_filtered(&SessionFilter::default(), None)
        .unwrap()
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert!(listed.contains(&id(CHILD)), "{listed:?}");
    wait_pass(&passes, true);
    assert!(tailer.shutdown(Duration::from_secs(10)));
    assert!(reader.session_creation(&id(CHILD)).unwrap().is_none());
    assert!(changes(&events).is_empty());
    let before = snapshot(&home);

    // Restarted with the reader unavailable: nothing is scanned or queued,
    // and no source changes. The sweep reads the indexed roots.
    let (tailer, events, passes) = worker(&database, &home, missing(), spawn_limits());
    wait_pass(&passes, false);
    let status = tailer.status();
    tailer.stop();
    assert_eq!(shown(&reader), [(id(CHILD), id(PARENT))]);
    // The relation, the child facts of the child and the Guardian, and the
    // display checks of the two main sessions, which the pass finished.
    assert_eq!(changes(&events), [5]);
    assert_eq!(status.reconciles, 1, "{status:?}");
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .all(|event| !matches!(event, TailEvent::Reconciled { .. }))
    );

    // Again: the sweep replays the relation and announces nothing.
    let (tailer, events, passes) = worker(&database, &home, missing(), spawn_limits());
    wait_pass(&passes, false);
    tailer.stop();
    assert_eq!(shown(&reader), [(id(CHILD), id(PARENT))]);
    assert!(changes(&events).is_empty());
    assert_eq!(snapshot(&home), before);

    // The child's opening header now names another indexed parent: the sweep
    // withholds the relation, once.
    write(&child, &exchange(&header(CHILD, spawned_by(OTHER))));
    let (tailer, events, passes) = worker(&database, &home, missing(), spawn_limits());
    wait_pass(&passes, false);
    tailer.stop();
    assert!(shown(&reader).is_empty());
    assert_eq!(changes(&events), [1]);
    let (stored, conflicted) = reader.session_creation(&id(CHILD)).unwrap().unwrap();
    assert_eq!(stored.parent_native_session_id, PARENT);
    assert!(conflicted);
    // The child is still known to be a child: only its parent is withheld.
    assert_eq!(
        reader
            .child_facts(&id(CHILD))
            .unwrap()
            .iter()
            .map(|(fact, accepted)| (fact.evidence_kind, *accepted))
            .collect::<Vec<_>>(),
        [(ChildEvidence::CodexThreadSpawn, true)]
    );

    // A relation already withheld stays so, silently.
    let (tailer, events, passes) = worker(&database, &home, missing(), spawn_limits());
    wait_pass(&passes, false);
    tailer.stop();
    assert!(shown(&reader).is_empty());
    assert!(changes(&events).is_empty());
    assert!(reader.session_creation(&id(GUARDIAN)).unwrap().is_none());
}

#[test]
fn reviewer_assumption_is_found_by_restart_sweep_after_old_bootstrap_completed() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let child = rollout_path(&home, CHILD);
    write(
        &child,
        &rollout(&header_with(
            CHILD,
            json!({"subagent":{"other":"guardian"}}),
            Some(json!(PARENT)),
        )),
    );
    let main = rollout_path(&home, OTHER);
    write(&main, &rollout(&header(OTHER, json!("cli"))));
    let mut db = TempDb::empty().unwrap();
    index(
        db.store_mut(),
        &[(CHILD, child.as_path()), (OTHER, main.as_path())],
    );
    db.store_mut()
        .advance_session_creation_bootstrap(
            CreationEvidence::CodexThreadSpawn,
            CODEX_THREAD_SPAWN_VERSION,
            &CreationBootstrap {
                after_locator: None,
                complete: true,
            },
        )
        .unwrap();
    let before = snapshot(&home);
    let mut backlog = SpawnBacklog::starting();
    let progress =
        continue_codex_spawns(db.store_mut(), &home, &mut backlog, spawn_limits(), None, T)
            .unwrap();
    // The reviewer origin and the Guardian's child fact.
    assert_eq!(progress.changed, 2);
    assert!(db.store().records(&id(CHILD)).unwrap()[0].human_excluded);
    assert!(!db.store().records(&id(CHILD)).unwrap()[0].confirmed_automated_input);
    assert!(!db.store().records(&id(OTHER)).unwrap()[0].human_excluded);
    assert!(shown(db.store()).is_empty());
    let mut replay = SpawnBacklog::starting();
    assert_eq!(
        continue_codex_spawns(db.store_mut(), &home, &mut replay, spawn_limits(), None, T)
            .unwrap()
            .changed,
        0
    );
    assert_eq!(snapshot(&home), before);
}
