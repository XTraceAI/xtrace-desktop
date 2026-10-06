//! Codex sessions a Codex agent started with `codex exec --json`
//! (`codex_cli_launch`): the launch scan's guarded writes record the child
//! fact with or without a parent and the relation under the existing checks;
//! the tables hold the new kind only with a Codex child; and a schema 19
//! index upgrades with every relation, fact, owner and launch row unchanged.

use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    child_fact::{ChildEvidence, ChildFact, ChildFactDisposition},
    claude_launch::{
        CandidateRow, ChildState, GroupStatus, LaunchCandidate, LaunchKey, MemberRecord,
        SegmentGeneration, SourceVerdict,
    },
    creation::{
        CLAUDE_LAUNCH_CREATE_VERSION, CODEX_CLI_LAUNCH_VERSION, CodexCliLaunchCreationProof,
        CreationAbstention, CreationDisposition, CreationEvidence, CreationWitness,
    },
    session_list::{self, ParentEvidence, SessionFilter},
};

const PARENT: &str = "01a00000-0000-7000-8000-0000000000aa";
const CHILD: &str = "01a10000-0000-7000-8000-0000000000c1";
const T: i64 = 1_791_238_000_000;

fn codex(native: &str) -> String {
    format!("codex-{native}")
}

fn first(native: &str) -> String {
    format!("{native}-first")
}

fn record(value: serde_json::Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}

fn session(store: &mut Store, native: &str, input: &str, at: &str) {
    let mut meta = SessionMeta::new(codex(native), "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some(native.into());
    meta.started_at_ms = Some(1);
    store.upsert_session(&meta, false).unwrap();
    store
        .upsert_records(
            &codex(native),
            &[record(
                json!({"uuid": input, "type": "user", "timestamp": at,
                "message": {"role": "user", "content": [{"type": "text", "text": "Synthetic"}]}}),
            )],
            false,
        )
        .unwrap();
}

fn member() -> MemberRecord {
    MemberRecord {
        rollout_id: PARENT.into(),
        generation: SegmentGeneration {
            device: 1,
            inode: 2,
            length: 4000,
            mtime_ns: 3,
            ctime_ns: 4,
        },
        scanned_length: 4000,
        valid: true,
    }
}

fn candidate(call: &str, host: Host) -> LaunchCandidate {
    LaunchCandidate {
        key: LaunchKey {
            parent_native_session_id: PARENT.into(),
            rollout_id: PARENT.into(),
            launch_call_id: call.into(),
            launch_operation_index: 0,
        },
        child_native_session_id: CHILD.into(),
        acknowledgment_call_id: call.into(),
        acknowledgment_operation_index: 0,
        process_session_id: Some("69773".into()),
        launch_offset: 100,
        acknowledgment_offset: 900,
        launch_ordinal: Some(2),
        acknowledgment_ordinal: Some(3),
        binding_call_offset: None,
        binding_output_offset: None,
        child_host: host,
        launch_check_fingerprint: Some("0".repeat(64)),
    }
}

/// Publish one validation of `candidates`; the published rows.
fn publish(store: &mut Store, candidates: &[LaunchCandidate]) -> Vec<CandidateRow> {
    let allocation = store.begin_claude_launch_validation(PARENT, 9).unwrap();
    assert!(
        store
            .stage_claude_launch_candidates(PARENT, allocation.allocated, candidates)
            .unwrap()
    );
    assert!(
        store
            .publish_claude_launch_validation(
                PARENT,
                allocation,
                GroupStatus::Valid,
                &[member()],
                candidates.len(),
            )
            .unwrap()
    );
    store
        .claude_launch_open_candidates(PARENT, None, 10)
        .unwrap()
}

fn fact(call: &str) -> ChildFact {
    ChildFact {
        child_session_id: codex(CHILD),
        child_host: Host::Codex,
        child_native_session_id: CHILD.into(),
        evidence_kind: ChildEvidence::CodexCliLaunch,
        evidence_version: CODEX_CLI_LAUNCH_VERSION,
        source_native_session_id: PARENT.into(),
        source_rollout_id: Some(PARENT.into()),
        launch_call_id: Some(call.into()),
        launch_operation_index: Some(0),
        first_record_uuid: Some(first(CHILD)),
    }
}

fn proof(call: &str) -> CodexCliLaunchCreationProof {
    CodexCliLaunchCreationProof {
        child_session_id: codex(CHILD),
        child_native_session_id: CHILD.into(),
        parent_session_id: codex(PARENT),
        parent_native_session_id: PARENT.into(),
        first_record_uuid: first(CHILD),
        launch_call_id: call.into(),
        launch_operation_index: 0,
        process_session_id: Some("69773".into()),
        acknowledgment_call_id: call.into(),
        acknowledgment_operation_index: 0,
        segment_rollout_id: PARENT.into(),
        launch_ordinal: Some(2),
        acknowledgment_ordinal: Some(3),
        evidence_version: CODEX_CLI_LAUNCH_VERSION,
    }
}

fn shown(store: &Store) -> Vec<(String, String, ParentEvidence)> {
    store
        .sessions_page_filtered(&SessionFilter::default(), None)
        .unwrap()
        .into_iter()
        .filter_map(|row| {
            let parent = row.parent?;
            Some((row.id, parent.session_id, parent.evidence))
        })
        .collect()
}

/// The child fact needs no parent: with none indexed it is recorded and
/// marks a known child; once the parent is indexed the relation is recorded
/// beside it, shown as an agent launch, and the Human input rule applies to
/// the child alone. A replay changes nothing.
#[test]
fn the_child_fact_stands_without_a_parent_and_the_relation_joins_it() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("codex.sqlite");
    let mut store = Store::open(&path).unwrap();
    session(&mut store, CHILD, &first(CHILD), "2026-10-05T22:17:31.936Z");
    let rows = publish(&mut store, &[candidate("call_launch", Host::Codex)]);
    let [row] = rows.as_slice() else {
        panic!("{rows:?}")
    };
    assert_eq!(row.candidate.child_host, Host::Codex);
    let facts = store
        .record_claude_launch_child_guarded(&fact("call_launch"), row, &[member()], T)
        .unwrap()
        .unwrap();
    assert_eq!(facts.dispositions, [ChildFactDisposition::Recorded]);
    let known = || {
        session_list::context(&Connection::open(&path).unwrap(), &[&codex(CHILD)])
            .unwrap()
            .remove(0)
            .known_child
    };
    assert!(known());
    assert!(shown(&store).is_empty());
    // Without the parent indexed the relation abstains, and the fact stays.
    let none = store
        .record_claude_launch_guarded(&proof("call_launch"), row, &[member()], T)
        .unwrap()
        .unwrap();
    assert_eq!(
        none.dispositions,
        [CreationDisposition::Abstained(
            CreationAbstention::ParentMismatch
        )]
    );
    assert!(known());
    assert_eq!(store.child_facts(&codex(CHILD)).unwrap().len(), 1);

    session(&mut store, PARENT, "parent-typed", "2026-10-05T22:00:00Z");
    let before = store.records(&codex(PARENT)).unwrap()[0].human_excluded;
    let linked = store
        .record_claude_launch_guarded(&proof("call_launch"), row, &[member()], T)
        .unwrap()
        .unwrap();
    assert_eq!(linked.dispositions, [CreationDisposition::Recorded]);
    let stored = store.session_creation(&codex(CHILD)).unwrap().unwrap().0;
    assert_eq!(
        (stored.evidence_kind, stored.witness, stored.child_host),
        (
            CreationEvidence::CodexCliLaunch,
            CreationWitness::CodexExecJsonThreadStarted,
            Host::Codex
        )
    );
    assert_eq!(
        store.codex_cli_launch_creation(&codex(CHILD)).unwrap(),
        Some((proof("call_launch"), false))
    );
    assert!(
        store
            .claude_launch_creation(&codex(CHILD))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        shown(&store),
        [(codex(CHILD), codex(PARENT), ParentEvidence::AgentLaunch)]
    );
    assert!(store.records(&codex(CHILD)).unwrap()[0].human_excluded);
    assert_eq!(
        store.records(&codex(PARENT)).unwrap()[0].human_excluded,
        before
    );
    // Linked now: a replay through the guard is refused, directly a no-op.
    assert!(
        store
            .record_claude_launch_guarded(&proof("call_launch"), row, &[member()], T)
            .unwrap()
            .is_none()
    );
}

/// The proof and fact must be the Codex candidate's own kind and version;
/// a Claude launch's version or kind for a Codex child is refused before
/// anything is written, and so is a Codex fact naming a Claude child.
#[test]
fn a_codex_launch_takes_only_its_own_kind_and_version() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(directory.path().join("codex.sqlite")).unwrap();
    session(&mut store, CHILD, &first(CHILD), "2026-10-05T22:17:31.936Z");
    session(&mut store, PARENT, "parent-typed", "2026-10-05T22:00:00Z");
    let rows = publish(&mut store, &[candidate("call_launch", Host::Codex)]);
    let row = &rows[0];
    let mut claude_version = proof("call_launch");
    claude_version.evidence_version = CLAUDE_LAUNCH_CREATE_VERSION;
    assert!(
        store
            .record_claude_launch_guarded(&claude_version, row, &[member()], T)
            .is_err()
    );
    let mut claude_kind = fact("call_launch");
    claude_kind.evidence_kind = ChildEvidence::CodexClaudeLaunch;
    claude_kind.evidence_version = CLAUDE_LAUNCH_CREATE_VERSION;
    assert!(
        store
            .record_claude_launch_child_guarded(&claude_kind, row, &[member()], T)
            .is_err()
    );
    let mut claude_child = fact("call_launch");
    claude_child.child_host = Host::Claude;
    assert!(store.record_child_facts(&[claude_child], T).is_err());
    let mut own = fact("call_launch");
    own.source_native_session_id = CHILD.into();
    assert!(store.record_child_facts(&[own], T).is_err());
    assert!(store.child_facts(&codex(CHILD)).unwrap().is_empty());
    assert!(store.session_creation(&codex(CHILD)).unwrap().is_none());
}

/// The tables themselves hold the new kind only with a Codex child and its
/// whole chain, and the old kinds only as before.
#[test]
fn the_tables_hold_the_new_kind_only_with_a_codex_child() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("codex.sqlite");
    let mut store = Store::open(&path).unwrap();
    session(&mut store, CHILD, &first(CHILD), "2026-10-05T22:17:31.936Z");
    drop(store);
    let connection = Connection::open(&path).unwrap();
    let relation = |child_host: &str, kind: &str, witness: &str, rollout: &str| {
        connection.execute(
            &format!(
                "INSERT INTO session_creation_relations(child_session_id,child_host,
                     child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
                     evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
                     launch_call_id,launch_operation_index,completion_call_id,
                     completion_operation_index,segment_rollout_id)
                 VALUES ('{child}','{child_host}','{CHILD}','codex','{PARENT}','{kind}',1,
                     '{witness}','accepted',1,'{parent}','{first}','call_launch',0,'call_launch',0,
                     '{rollout}')",
                child = codex(CHILD),
                parent = codex(PARENT),
                first = first(CHILD),
            ),
            [],
        )
    };
    for (host, kind, witness, rollout) in [
        (
            "claude",
            "codex_cli_launch",
            "codex_exec_json_thread_started",
            PARENT,
        ),
        (
            "codex",
            "codex_cli_launch",
            "codex_exec_session_id_launch",
            PARENT,
        ),
        (
            "codex",
            "codex_claude_launch",
            "codex_exec_session_id_launch",
            PARENT,
        ),
        (
            "codex",
            "codex_claude_launch",
            "codex_exec_json_thread_started",
            PARENT,
        ),
        (
            "codex",
            "codex_cli_launch",
            "codex_exec_json_thread_started",
            "x",
        ),
        (
            "codex",
            "codex_new_kind",
            "codex_exec_json_thread_started",
            PARENT,
        ),
    ] {
        assert!(
            relation(host, kind, witness, rollout).is_err(),
            "{host} {kind} {witness} {rollout}"
        );
    }
    let fact = |child_host: &str, kind: &str, rollout: Option<&str>| {
        connection.execute(
            "INSERT INTO session_child_facts(child_session_id,child_host,child_native_session_id,
                 evidence_kind,evidence_version,source_native_session_id,source_rollout_id,
                 launch_call_id,launch_operation_index,first_record_uuid,state,recorded_at)
             VALUES (?1,?2,?3,?4,1,?5,?6,'call_launch',0,?7,'accepted',1)",
            rusqlite::params![
                codex(CHILD),
                child_host,
                CHILD,
                kind,
                PARENT,
                rollout,
                first(CHILD)
            ],
        )
    };
    for (host, kind, rollout) in [
        ("claude", "codex_cli_launch", Some(PARENT)),
        ("codex", "codex_cli_launch", None),
        ("codex", "codex_claude_launch", Some(PARENT)),
        ("codex", "claude_bash_launch", None),
    ] {
        assert!(fact(host, kind, rollout).is_err(), "{host} {kind}");
    }
    // The new kind with its whole chain fits both tables.
    assert_eq!(fact("codex", "codex_cli_launch", Some(PARENT)).unwrap(), 1);
}

/// A schema 19 index — every older relation kind, child fact kind, launch
/// owner and launch row — upgrades with each row, its row identity and its
/// readers' results unchanged; launch rows read as naming Claude children;
/// the rebuilt tables keep their indexes and immutability triggers.
#[test]
fn a_schema_19_index_upgrades_with_every_row_unchanged() {
    const MIGRATIONS: [&str; 19] = [
        include_str!("../migrations/0001_canonical.sql"),
        include_str!("../migrations/0002_ingest.sql"),
        include_str!("../migrations/0003_native_checkpoints.sql"),
        include_str!("../migrations/0004_native_record_copies.sql"),
        include_str!("../migrations/0005_human_classification_replay.sql"),
        include_str!("../migrations/0006_structural_tool_kinds.sql"),
        include_str!("../migrations/0007_native_pr_witnesses.sql"),
        include_str!("../migrations/0008_pr_refresh_status.sql"),
        include_str!("../migrations/0009_repeat_group_keys.sql"),
        include_str!("../migrations/0010_confirmed_automated_inputs.sql"),
        include_str!("../migrations/0011_session_creation_relations.sql"),
        include_str!("../migrations/0012_guardian_turn_inputs.sql"),
        include_str!("../migrations/0013_injected_context_inputs.sql"),
        include_str!("../migrations/0014_human_input_estimate.sql"),
        include_str!("../migrations/0015_claude_launch_creations.sql"),
        include_str!("../migrations/0016_task_notification_inputs.sql"),
        include_str!("../migrations/0017_record_previews.sql"),
        include_str!("../migrations/0018_session_child_facts.sql"),
        include_str!("../migrations/0019_claude_launch_binding_witnesses.sql"),
    ];
    const VIEWS: [&str; 5] = [
        include_str!("../views/human_inputs.sql"),
        include_str!("../views/records.sql"),
        include_str!("../views/usage_records.sql"),
        include_str!("../views/response_usage.sql"),
        include_str!("../views/session_events.sql"),
    ];
    const SPAWNED: &str = "01a00000-0000-7000-8000-0000000000bb";
    const GUARDIAN: &str = "01a00000-0000-7000-8000-0000000000dd";
    const CLAUDE_A: &str = "0c000000-0000-4000-8000-0000000000a1";
    const CLAUDE_B: &str = "0c000000-0000-4000-8000-0000000000b1";
    const CLAUDE_C: &str = "0c000000-0000-4000-8000-0000000000c3";
    const ROLLOUT: &str = "01a00000-0000-7000-8000-0000000000b3";
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("old.sqlite");
    {
        let connection = Connection::open(&path).unwrap();
        xt_store::timestamp::register_sqlite(&connection).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)",
            )
            .unwrap();
        for (index, sql) in MIGRATIONS.iter().enumerate() {
            connection.execute_batch(sql).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_version VALUES (?1,'2026-10-04T00:00:00Z')",
                    [index as i64 + 1],
                )
                .unwrap();
        }
        // Today's views name the table migration 21 adds; a stand-in lets
        // them read this older index until the upgrade below replaces it.
        connection
            .execute_batch("CREATE TABLE tool_sent_inputs(record_uuid TEXT, session_id TEXT)")
            .unwrap();
        for view in VIEWS {
            connection.execute_batch(view).unwrap();
        }
        for (id, host, native) in [
            (codex(PARENT), "codex", PARENT),
            (codex(SPAWNED), "codex", SPAWNED),
            (codex(GUARDIAN), "codex", GUARDIAN),
            (CLAUDE_A.into(), "claude", CLAUDE_A),
            (CLAUDE_B.into(), "claude", CLAUDE_B),
            (CLAUDE_C.into(), "claude", CLAUDE_C),
        ] {
            connection
                .execute(
                    "INSERT INTO sessions(session_id,host,native_session_id,kind,source)
                     VALUES (?1,?2,?3,'user','readers_cli')",
                    [id, host.to_owned(), native.to_owned()],
                )
                .unwrap();
        }
        for session in [CLAUDE_A, CLAUDE_B, CLAUDE_C] {
            connection
                .execute(
                    "INSERT INTO records(uuid,session_id,type,ts,ts_ms,role,is_meta,is_sidechain,
                         is_tool_result_carrier,text_len,is_human)
                     VALUES (?1,?2,'user','2026-09-29T07:58:40.536Z',1790668720536,'user',0,0,0,5,1)",
                    [first(session), session.to_owned()],
                )
                .unwrap();
        }
        let parent = codex(PARENT);
        connection
            .execute_batch(&format!(
                "INSERT INTO session_creation_relations(child_session_id,child_host,
                     child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
                     evidence_version,witness,state,recorded_at)
                 VALUES ('codex-{SPAWNED}','codex','{SPAWNED}','codex','{PARENT}',
                     'codex_thread_spawn',2,'rollout_opening_session_meta','accepted',5);
                 INSERT INTO session_creation_relations(child_session_id,child_host,
                     child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
                     evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
                     launch_call_id,launch_operation_index,process_session_id,completion_call_id,
                     output_read_call_id,provider_result_uuid)
                 VALUES ('{CLAUDE_A}','claude','{CLAUDE_A}','codex','{PARENT}','cli_artifact_create',1,
                     'claude_cli_redirected_json_result','accepted',6,'{parent}','{CLAUDE_A}-first',
                     'call_a',0,'21036','call_c','call_r','0d000000-0000-4000-8000-000000000001');
                 INSERT INTO session_creation_relations(child_session_id,child_host,
                     child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
                     evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
                     launch_call_id,launch_operation_index,process_session_id,completion_call_id,
                     completion_operation_index,segment_rollout_id,launch_ordinal,completion_ordinal)
                 VALUES ('{CLAUDE_B}','claude','{CLAUDE_B}','codex','{PARENT}','codex_claude_launch',6,
                     'codex_exec_session_id_launch','conflicted',7,'{parent}','{CLAUDE_B}-first',
                     'call_b',1,'56751','call_b',1,'{ROLLOUT}',10,12);
                 INSERT INTO cli_artifact_launch_owners VALUES ('{parent}','call_a',0,'{CLAUDE_A}',0);
                 INSERT INTO cli_artifact_launch_owners VALUES ('{parent}','call_b',1,'{CLAUDE_B}',1);
                 INSERT INTO session_child_facts(child_session_id,child_host,child_native_session_id,
                     evidence_kind,evidence_version,source_native_session_id,state,recorded_at)
                 VALUES ('codex-{SPAWNED}','codex','{SPAWNED}','codex_thread_spawn',2,'{SPAWNED}',
                     'accepted',8),
                    ('codex-{GUARDIAN}','codex','{GUARDIAN}','codex_guardian',1,'{GUARDIAN}',
                     'accepted',9);
                 INSERT INTO session_child_facts(child_session_id,child_host,child_native_session_id,
                     evidence_kind,evidence_version,source_native_session_id,source_rollout_id,
                     launch_call_id,launch_operation_index,first_record_uuid,state,recorded_at)
                 VALUES ('{CLAUDE_B}','claude','{CLAUDE_B}','codex_claude_launch',6,'{PARENT}',
                     '{ROLLOUT}','call_b',1,'{CLAUDE_B}-first','withheld',10),
                    ('{CLAUDE_C}','claude','{CLAUDE_C}','claude_bash_launch',2,'{CLAUDE_A}',NULL,
                     'toolu_1',0,'{CLAUDE_C}-first','accepted',11);
                 INSERT INTO claude_launch_groups VALUES ('{PARENT}',3,3,'valid',8);
                 INSERT INTO claude_launch_group_members VALUES ('{PARENT}','{ROLLOUT}',3,1,2,4000,3,4,
                     4000,'valid');
                 INSERT INTO claude_launch_candidates(parent_native_session_id,rollout_id,launch_call_id,
                     launch_operation_index,child_native_session_id,completion_call_id,
                     completion_operation_index,process_session_id,launch_offset,completion_offset,
                     launch_ordinal,completion_ordinal,source_revision,source_verdict,child_state,
                     binding_call_offset,binding_output_offset)
                 VALUES ('{PARENT}','{ROLLOUT}','call_b',1,'{CLAUDE_B}','call_b',1,'56751',100,900,
                     10,12,3,'valid','linked',NULL,NULL),
                    ('{PARENT}','{ROLLOUT}','call_d',0,'{CLAUDE_C}','call_d',0,NULL,1000,1900,
                     20,21,3,'valid','waiting',10,50);
                 INSERT INTO claude_launch_staged_candidates(parent_native_session_id,revision,
                     rollout_id,launch_call_id,launch_operation_index,child_native_session_id,
                     completion_call_id,completion_operation_index,process_session_id,launch_offset,
                     completion_offset,launch_ordinal,completion_ordinal)
                 VALUES ('{PARENT}',4,'{ROLLOUT}','call_e',0,'{CLAUDE_C}','call_e',0,NULL,2000,2100,
                     NULL,NULL);"
            ))
            .unwrap();
    }
    let dump = || -> Vec<String> {
        let connection = Connection::open(&path).unwrap();
        [
            "SELECT json_group_array(json_array(rowid,child_session_id,child_host,
                 child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
                 evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
                 launch_call_id,launch_operation_index,process_session_id,completion_call_id,
                 output_read_call_id,provider_result_uuid,completion_operation_index,
                 segment_rollout_id,launch_ordinal,completion_ordinal))
             FROM (SELECT rowid,* FROM session_creation_relations ORDER BY rowid)",
            "SELECT json_group_array(json_array(rowid,child_session_id,child_host,
                 child_native_session_id,evidence_kind,evidence_version,source_native_session_id,
                 source_rollout_id,launch_call_id,launch_operation_index,first_record_uuid,state,
                 recorded_at))
             FROM (SELECT rowid,* FROM session_child_facts ORDER BY rowid)",
            "SELECT json_group_array(json_array(parent_session_id,launch_call_id,
                 launch_operation_index,first_child_session_id,disputed))
             FROM (SELECT * FROM cli_artifact_launch_owners ORDER BY 1,2,3)",
            "SELECT json_group_array(json_array(parent_native_session_id,rollout_id,launch_call_id,
                 launch_operation_index,child_native_session_id,completion_call_id,
                 completion_operation_index,process_session_id,launch_offset,completion_offset,
                 launch_ordinal,completion_ordinal,source_revision,source_verdict,child_state,
                 binding_call_offset,binding_output_offset))
             FROM (SELECT * FROM claude_launch_candidates ORDER BY 1,2,3,4)",
            "SELECT json_group_array(json_array(parent_native_session_id,revision,rollout_id,
                 launch_call_id,child_native_session_id,binding_call_offset))
             FROM (SELECT * FROM claude_launch_staged_candidates ORDER BY 1,2,3,4)",
            "SELECT json_group_array(json_array(uuid,human_is_eligible))
             FROM (SELECT * FROM v_human_inputs ORDER BY uuid)",
        ]
        .into_iter()
        .map(|query| connection.query_row(query, [], |row| row.get(0)).unwrap())
        .collect()
    };
    let before = dump();
    Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TABLE tool_sent_inputs")
        .unwrap();
    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 22);
        assert_eq!(dump(), before);
        let parents = shown(&store);
        assert!(parents.contains(&(codex(SPAWNED), codex(PARENT), ParentEvidence::NativeSpawn)));
        assert!(parents.contains(&(CLAUDE_A.into(), codex(PARENT), ParentEvidence::AgentLaunch)));
        assert_eq!(parents.len(), 2, "{parents:?}");
        assert_eq!(
            store
                .child_facts(&codex(GUARDIAN))
                .unwrap()
                .into_iter()
                .map(|(fact, accepted)| (fact.evidence_kind, accepted))
                .collect::<Vec<_>>(),
            [(ChildEvidence::CodexGuardian, true)]
        );
        let rows = store
            .claude_launch_candidates_for_child(CLAUDE_B, None, 10)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].candidate.child_host, Host::Claude);
        assert_eq!(rows[0].source_verdict, SourceVerdict::Valid);
        assert_eq!(rows[0].child_state, ChildState::Linked);
    }
    let connection = Connection::open(&path).unwrap();
    let hosts: Vec<String> = connection
        .prepare(
            "SELECT child_host FROM claude_launch_candidates UNION ALL
             SELECT child_host FROM claude_launch_staged_candidates",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(hosts, ["claude", "claude", "claude"]);
    let names: Vec<String> = connection
        .prepare(
            "SELECT name FROM sqlite_schema WHERE tbl_name IN
                 ('session_creation_relations','session_child_facts')
               AND type IN ('index','trigger') AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        names,
        [
            "session_child_facts_anchor",
            "session_child_facts_immutable_delete",
            "session_child_facts_immutable_update",
            "session_child_facts_launch",
            "session_creation_relations_immutable_delete",
            "session_creation_relations_immutable_update",
            "session_creation_relations_launch",
            "session_creation_relations_parent",
        ]
    );
    for statement in [
        "UPDATE session_creation_relations SET parent_native_session_id='x'",
        "DELETE FROM session_creation_relations",
        "UPDATE session_child_facts SET source_native_session_id='x'",
        "DELETE FROM session_child_facts",
        "UPDATE claude_launch_candidates SET child_host='other'",
    ] {
        assert!(connection.execute(statement, []).is_err(), "{statement}");
    }
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
}
