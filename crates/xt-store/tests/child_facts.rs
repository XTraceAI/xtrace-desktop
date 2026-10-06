//! Child facts: that an indexed session was created by an agent, kept apart
//! from who that agent was. A fact binds the exact canonical child, host and
//! native identity (one indexed user session) and, for a launch, the child's
//! own first input; independent facts for one child stand side by side; a
//! launch claimed for two children, or with two first inputs, withholds only
//! the facts on that launch; replay changes nothing; nothing stored is
//! rewritten. A fact sets the known-child bit, with or without a parent, and
//! leaves the Human input view, relations and saved origins exactly as they
//! were. Synthetic sessions only.

use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    child_fact::{ChildEvidence, ChildFact, ChildFactDisposition},
    creation::{
        CODEX_THREAD_SPAWN_VERSION, CreationAbstention, CreationEvidence, CreationWitness,
        SessionCreationProof,
    },
    human_input::{OriginManifest, SessionOrigin},
    session_list,
};

const CODEX_PARENT: &str = "019a0000-0000-7000-8000-0000000000aa";
const CODEX_CHILD: &str = "019a0000-0000-7000-8000-0000000000bb";
const CODEX_OTHER: &str = "019a0000-0000-7000-8000-0000000000cc";
const CLAUDE_CHILD: &str = "0c000000-0000-4000-8000-0000000000c1";
const CLAUDE_OTHER: &str = "0c000000-0000-4000-8000-0000000000c2";
const CALLER: &str = "0c000000-0000-4000-8000-0000000000ca";
const ROLLOUT: &str = "019a0000-0000-7000-8000-0000000000aa";

fn path(directory: &TempDir) -> std::path::PathBuf {
    directory.path().join("child-facts.sqlite")
}

fn sql(directory: &TempDir) -> Connection {
    Connection::open(path(directory)).unwrap()
}

fn canonical(host: Host, native: &str) -> String {
    format!("{}-{native}", host.as_str())
}

/// An untitled session with one user message, its first input.
fn session(store: &mut Store, host: Host, native: &str) -> String {
    let id = canonical(host, native);
    let source = if host == Host::Codex {
        SessionSource::ReadersCli
    } else {
        SessionSource::Transcript
    };
    let mut meta = SessionMeta::new(id.clone(), host.as_str(), source);
    meta.native_session_id = Some(native.into());
    store.upsert_session(&meta, false).unwrap();
    let input: CanonicalRecord = serde_json::from_value(json!({
        "uuid": format!("input-{native}"), "type": "user",
        "timestamp": "2026-09-01T00:00:00Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "Synthetic"}]}
    }))
    .unwrap();
    store.upsert_records(&id, &[input], false).unwrap();
    id
}

fn typed(native: &str, kind: ChildEvidence) -> ChildFact {
    ChildFact::typed(&canonical(Host::Codex, native), native, kind)
}

/// A launch fact for a Claude child: a Codex `exec` launch, or a Claude
/// `Bash` call, from `source` at `call`.
fn launch(kind: ChildEvidence, child: &str, source: &str, call: &str) -> ChildFact {
    ChildFact {
        child_session_id: canonical(Host::Claude, child),
        child_host: Host::Claude,
        child_native_session_id: child.into(),
        evidence_kind: kind,
        evidence_version: kind.version(),
        source_native_session_id: source.into(),
        source_rollout_id: (kind == ChildEvidence::CodexClaudeLaunch).then(|| ROLLOUT.into()),
        launch_call_id: Some(call.into()),
        launch_operation_index: Some(0),
        first_record_uuid: Some(format!("input-{child}")),
    }
}

fn record(store: &mut Store, facts: &[ChildFact]) -> Vec<ChildFactDisposition> {
    store.record_child_facts(facts, 7).unwrap().dispositions
}

fn known(directory: &TempDir, id: &str) -> bool {
    session_list::context(&sql(directory), &[id]).unwrap()[0].known_child
}

fn parent(directory: &TempDir, id: &str) -> Option<String> {
    session_list::context(&sql(directory), &[id]).unwrap()[0]
        .parent
        .as_ref()
        .map(|parent| parent.session_id.clone())
}

/// Whether the Human input view excludes the session's one user message.
fn human_excluded(store: &Store, id: &str) -> bool {
    store.records(id).unwrap()[0].human_excluded
}

/// Every stored relation and origin row, whole.
fn relations_and_origins(directory: &TempDir) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let rows = |query: &str| {
        let connection = sql(directory);
        let mut statement = connection.prepare(query).unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| {
                (0..columns)
                    .map(|index| {
                        row.get::<_, rusqlite::types::Value>(index)
                            .map(|value| format!("{value:?}"))
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    (
        rows("SELECT * FROM session_creation_relations ORDER BY child_session_id"),
        rows("SELECT * FROM human_session_origins ORDER BY session_id"),
    )
}

#[test]
fn a_typed_fact_marks_a_known_child_without_a_parent_or_a_human_change() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(path(&directory)).unwrap();
    let child = session(&mut store, Host::Codex, CODEX_CHILD);
    let guardian = session(&mut store, Host::Codex, CODEX_OTHER);
    assert!(!known(&directory, &child));
    assert_eq!(
        record(
            &mut store,
            &[
                typed(CODEX_CHILD, ChildEvidence::CodexThreadSpawn),
                typed(CODEX_OTHER, ChildEvidence::CodexGuardian),
            ]
        ),
        [
            ChildFactDisposition::Recorded,
            ChildFactDisposition::Recorded
        ]
    );
    for id in [&child, &guardian] {
        assert!(known(&directory, id));
        assert_eq!(parent(&directory, id), None);
        // The Human input view does not read child facts.
        assert!(!human_excluded(&store, id));
    }
    // A replay changes nothing; an independent kind for the same child is
    // kept beside it.
    let report = store
        .record_child_facts(
            &[
                typed(CODEX_CHILD, ChildEvidence::CodexThreadSpawn),
                typed(CODEX_CHILD, ChildEvidence::CodexGuardian),
            ],
            8,
        )
        .unwrap();
    assert_eq!(
        report.dispositions,
        [
            ChildFactDisposition::AlreadyRecorded,
            ChildFactDisposition::Recorded
        ]
    );
    assert_eq!(report.changed, 1);
    assert_eq!(store.child_facts(&child).unwrap().len(), 2);
    let summary = store.child_fact_summary().unwrap();
    assert_eq!(summary.children, 2);
    assert_eq!(
        summary.kinds,
        [
            ("codex_guardian".to_owned(), 2, 0),
            ("codex_thread_spawn".to_owned(), 1, 0)
        ]
    );
}

#[test]
fn a_fact_binds_one_exact_indexed_child_and_its_own_first_input() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(path(&directory)).unwrap();
    session(&mut store, Host::Codex, CODEX_CHILD);
    session(&mut store, Host::Claude, CLAUDE_CHILD);
    let mut wrong_native = typed(CODEX_CHILD, ChildEvidence::CodexThreadSpawn);
    wrong_native.child_native_session_id = CODEX_OTHER.into();
    wrong_native.source_native_session_id = CODEX_OTHER.into();
    let missing = typed(CODEX_OTHER, ChildEvidence::CodexThreadSpawn);
    let mut wrong_host = launch(
        ChildEvidence::ClaudeBashLaunch,
        CLAUDE_CHILD,
        CALLER,
        "toolu_1",
    );
    wrong_host.child_session_id = canonical(Host::Codex, CODEX_CHILD);
    let mut not_first = launch(
        ChildEvidence::ClaudeBashLaunch,
        CLAUDE_CHILD,
        CALLER,
        "toolu_2",
    );
    not_first.first_record_uuid = Some("not-a-record".into());
    assert_eq!(
        record(&mut store, &[wrong_native, missing, wrong_host, not_first]),
        [
            ChildFactDisposition::Abstained(CreationAbstention::ChildIdentityMismatch),
            ChildFactDisposition::Abstained(CreationAbstention::MissingChild),
            ChildFactDisposition::Abstained(CreationAbstention::ChildIdentityMismatch),
            ChildFactDisposition::Abstained(CreationAbstention::FirstInputMismatch),
        ]
    );
    // Two user sessions hold one native identity: no child is known.
    let mut twin = SessionMeta::new("plugin-twin", "codex", SessionSource::Plugin);
    twin.native_session_id = Some(CODEX_CHILD.into());
    store.upsert_session(&twin, false).unwrap();
    assert_eq!(
        record(
            &mut store,
            &[typed(CODEX_CHILD, ChildEvidence::CodexThreadSpawn)]
        ),
        [ChildFactDisposition::Abstained(
            CreationAbstention::AmbiguousChild
        )]
    );
    assert!(!known(&directory, &canonical(Host::Codex, CODEX_CHILD)));
    assert!(!known(&directory, &canonical(Host::Claude, CLAUDE_CHILD)));
    // A fact that does not fit its kind or version rejects the whole call.
    let mut versioned = typed(CODEX_CHILD, ChildEvidence::CodexThreadSpawn);
    versioned.evidence_version = CODEX_THREAD_SPAWN_VERSION + 1;
    let mut typed_launch = typed(CODEX_CHILD, ChildEvidence::CodexGuardian);
    typed_launch.launch_call_id = Some("call".into());
    let mut self_launch = launch(
        ChildEvidence::ClaudeBashLaunch,
        CLAUDE_CHILD,
        CLAUDE_CHILD,
        "toolu_3",
    );
    self_launch.source_native_session_id = CLAUDE_CHILD.into();
    let mut no_rollout = launch(
        ChildEvidence::CodexClaudeLaunch,
        CLAUDE_CHILD,
        CODEX_PARENT,
        "call_1",
    );
    no_rollout.source_rollout_id = None;
    let mut blank = launch(
        ChildEvidence::ClaudeBashLaunch,
        CLAUDE_CHILD,
        CALLER,
        "toolu 4",
    );
    blank.launch_call_id = Some("toolu 4".into());
    for fact in [versioned, typed_launch, self_launch, no_rollout, blank] {
        assert!(
            store
                .record_child_facts(
                    &[
                        launch(ChildEvidence::ClaudeBashLaunch, CLAUDE_CHILD, CALLER, "ok"),
                        fact.clone()
                    ],
                    9
                )
                .is_err(),
            "{fact:?}"
        );
    }
    assert!(
        store
            .child_facts(&canonical(Host::Claude, CLAUDE_CHILD))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_disputed_launch_withholds_only_its_own_facts() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(path(&directory)).unwrap();
    let child = session(&mut store, Host::Claude, CLAUDE_CHILD);
    let other = session(&mut store, Host::Claude, CLAUDE_OTHER);
    // Two independent creators of one child: both kept, neither contradicts.
    let cli = launch(
        ChildEvidence::CodexClaudeLaunch,
        CLAUDE_CHILD,
        CODEX_PARENT,
        "call_1",
    );
    let bash = launch(
        ChildEvidence::ClaudeBashLaunch,
        CLAUDE_CHILD,
        CALLER,
        "toolu_1",
    );
    assert_eq!(
        record(&mut store, &[cli.clone(), bash.clone()]),
        [
            ChildFactDisposition::Recorded,
            ChildFactDisposition::Recorded
        ]
    );
    assert!(known(&directory, &child));
    assert_eq!(parent(&directory, &child), None);
    // The same Codex launch claimed for a second child: every fact on it is
    // withheld, the second child stays unknown, and the first is still known
    // by its independent Bash fact.
    let second = launch(
        ChildEvidence::CodexClaudeLaunch,
        CLAUDE_OTHER,
        CODEX_PARENT,
        "call_1",
    );
    let report = store
        .record_child_facts(std::slice::from_ref(&second), 8)
        .unwrap();
    assert_eq!(report.dispositions, [ChildFactDisposition::Withheld]);
    assert_eq!(report.changed, 1);
    assert!(!known(&directory, &other));
    assert!(known(&directory, &child));
    let states: Vec<(ChildEvidence, bool)> = store
        .child_facts(&child)
        .unwrap()
        .into_iter()
        .map(|(fact, accepted)| (fact.evidence_kind, accepted))
        .collect();
    assert_eq!(
        states,
        [
            (ChildEvidence::CodexClaudeLaunch, false),
            (ChildEvidence::ClaudeBashLaunch, true)
        ]
    );
    // For good: a replay of either claim stays withheld.
    assert_eq!(
        record(&mut store, &[cli, second]),
        [
            ChildFactDisposition::Withheld,
            ChildFactDisposition::Withheld
        ]
    );
    // The same Bash launch with another first input for its child.
    let mut moved = bash.clone();
    moved.first_record_uuid = Some("input-elsewhere".into());
    assert_eq!(
        record(&mut store, &[moved]),
        [ChildFactDisposition::Abstained(
            CreationAbstention::FirstInputMismatch
        )]
    );
    // A detector that can no longer tell the child withholds that launch.
    assert_eq!(
        store
            .withhold_child_launch(ChildEvidence::ClaudeBashLaunch, CALLER, None, "toolu_1", 0)
            .unwrap(),
        1
    );
    assert!(!known(&directory, &child));
    assert_eq!(
        record(&mut store, &[bash]),
        [ChildFactDisposition::Withheld]
    );
    assert!(!known(&directory, &child));
}

#[test]
fn facts_are_immutable_and_leave_relations_origins_and_human_input_alone() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(path(&directory)).unwrap();
    session(&mut store, Host::Codex, CODEX_PARENT);
    let child = session(&mut store, Host::Codex, CODEX_CHILD);
    let guardian = session(&mut store, Host::Codex, CODEX_OTHER);
    store
        .record_session_creations(
            &[SessionCreationProof {
                child_session_id: child.clone(),
                child_host: Host::Codex,
                child_native_session_id: CODEX_CHILD.into(),
                parent_host: Host::Codex,
                parent_native_session_id: CODEX_PARENT.into(),
                evidence_kind: CreationEvidence::CodexThreadSpawn,
                evidence_version: 1,
                witness: CreationWitness::RolloutOpeningSessionMeta,
            }],
            1,
        )
        .unwrap();
    store
        .import_human_session_origins(&OriginManifest {
            version: 1,
            sessions: vec![SessionOrigin {
                session_id: guardian.clone(),
                host: Host::Codex,
                native_session_id: CODEX_OTHER.into(),
                parent_host: Host::Codex,
                parent_native_session_id: CODEX_PARENT.into(),
                method: "native_reviewer_header".into(),
                evidence_id: "rollout_opening_session_meta".into(),
                launch_id: "native_reviewer_header".into(),
            }],
        })
        .unwrap();
    let before = relations_and_origins(&directory);
    let excluded = (
        human_excluded(&store, &child),
        human_excluded(&store, &guardian),
    );
    assert_eq!(excluded, (true, true));
    record(
        &mut store,
        &[
            typed(CODEX_CHILD, ChildEvidence::CodexThreadSpawn),
            typed(CODEX_OTHER, ChildEvidence::CodexGuardian),
        ],
    );
    assert_eq!(relations_and_origins(&directory), before);
    assert_eq!(
        (
            human_excluded(&store, &child),
            human_excluded(&store, &guardian)
        ),
        excluded
    );
    assert_eq!(
        parent(&directory, &child),
        Some(canonical(Host::Codex, CODEX_PARENT))
    );
    let connection = sql(&directory);
    for statement in [
        "UPDATE session_child_facts SET evidence_version=evidence_version+1",
        "UPDATE session_child_facts SET source_native_session_id='x'",
        "UPDATE session_child_facts SET state='withheld',recorded_at=recorded_at+1",
        "DELETE FROM session_child_facts",
    ] {
        assert!(connection.execute(statement, []).is_err(), "{statement}");
    }
    connection
        .execute("UPDATE session_child_facts SET state='withheld'", [])
        .unwrap();
    assert!(
        connection
            .execute("UPDATE session_child_facts SET state='accepted'", [])
            .is_err()
    );
    // Only identities and closed labels are stored.
    let columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('session_child_facts') ORDER BY cid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        columns,
        [
            "child_session_id",
            "child_host",
            "child_native_session_id",
            "evidence_kind",
            "evidence_version",
            "source_native_session_id",
            "source_rollout_id",
            "launch_call_id",
            "launch_operation_index",
            "first_record_uuid",
            "state",
            "recorded_at"
        ]
    );
}
