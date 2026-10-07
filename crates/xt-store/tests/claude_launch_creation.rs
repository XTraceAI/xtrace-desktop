//! Native Codex launch sub-sessions (`codex_claude_launch`): the store
//! re-checks every identity it holds, an exact replay is silent, any
//! different claim on a child or on a launch — of either Claude CLI kind —
//! withholds what it contradicts, the parent shown is the exact canonical
//! one, the existing Human-input rule applies to the new child only, and a
//! schema 14 index upgrades with every relation it held unchanged.

use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, Store,
    creation::{
        CLAUDE_LAUNCH_CREATE_VERSION, CLI_ARTIFACT_CREATE_VERSION, ClaudeLaunchCreationProof,
        CliArtifactCreationProof, CreationAbstention, CreationDisposition, CreationEvidence,
        CreationWitness,
    },
    session_list::{ParentEvidence, SessionFilter},
};

const PARENT: &str = "019a0000-0000-7000-8000-0000000000aa";
const OTHER_PARENT: &str = "019a0000-0000-7000-8000-0000000000ab";
const CHILD: &str = "0c000000-0000-4000-8000-0000000000c1";
const SECOND: &str = "0c000000-0000-4000-8000-0000000000c2";
const ROLLOUT: &str = "019a0000-0000-7000-8000-0000000000b3";
const T: i64 = 1_790_700_000_000;

fn codex(native: &str) -> String {
    format!("codex-{native}")
}

fn first(child: &str) -> String {
    format!("{child}-first")
}

fn record(value: serde_json::Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}

fn parent_session(store: &mut Store, native: &str) {
    let mut meta = SessionMeta::new(codex(native), "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some(native.into());
    meta.started_at_ms = Some(1);
    store.upsert_session(&meta, false).unwrap();
    store
        .upsert_records(
            &codex(native),
            &[record(
                json!({"uuid": format!("{native}-typed"), "type": "user",
                "timestamp": "2026-09-29T07:00:00Z",
                "message": {"role": "user", "content": [{"type": "text", "text": "Typed"}]}}),
            )],
            false,
        )
        .unwrap();
}

fn child_session(store: &mut Store, native: &str) {
    let mut meta = SessionMeta::new(native, "claude", SessionSource::Transcript);
    meta.native_session_id = Some(native.into());
    meta.started_at_ms = Some(2);
    store.upsert_session(&meta, false).unwrap();
    let text = |uuid: String, at: &str| {
        record(json!({"uuid": uuid, "type": "user", "timestamp": at,
            "message": {"role": "user", "content": [{"type": "text", "text": "Synthetic input"}]}}))
    };
    store
        .upsert_records(
            native,
            &[
                text(first(native), "2026-09-29T07:58:40.536Z"),
                text(format!("{native}-later"), "2026-09-29T08:43:30Z"),
            ],
            false,
        )
        .unwrap();
}

fn seed(store: &mut Store) {
    parent_session(store, PARENT);
    parent_session(store, OTHER_PARENT);
    child_session(store, CHILD);
    child_session(store, SECOND);
}

fn proof(child: &str) -> ClaudeLaunchCreationProof {
    ClaudeLaunchCreationProof {
        child_session_id: child.into(),
        child_native_session_id: child.into(),
        parent_session_id: codex(PARENT),
        parent_native_session_id: PARENT.into(),
        first_record_uuid: first(child),
        launch_call_id: format!("call_launch_{}", &child[child.len() - 2..]),
        launch_operation_index: 0,
        process_session_id: Some("56751".into()),
        acknowledgment_call_id: "call_completion".into(),
        acknowledgment_operation_index: 0,
        segment_rollout_id: ROLLOUT.into(),
        launch_ordinal: Some(3546),
        acknowledgment_ordinal: Some(3939),
        evidence_version: CLAUDE_LAUNCH_CREATE_VERSION,
    }
}

fn artifact(child: &str, launch: &str) -> CliArtifactCreationProof {
    CliArtifactCreationProof {
        child_session_id: child.into(),
        child_native_session_id: child.into(),
        parent_session_id: codex(PARENT),
        parent_native_session_id: PARENT.into(),
        first_record_uuid: first(child),
        launch_call_id: launch.into(),
        launch_operation_index: 0,
        process_session_id: "21036".into(),
        completion_call_id: "call_completion".into(),
        output_read_call_id: "call_read".into(),
        provider_result_uuid: "0d000000-0000-4000-8000-000000000001".into(),
        evidence_version: CLI_ARTIFACT_CREATE_VERSION,
    }
}

fn apply(
    store: &mut Store,
    proofs: &[ClaudeLaunchCreationProof],
) -> (Vec<CreationDisposition>, usize) {
    let report = store.record_claude_launch_creations(proofs, T).unwrap();
    (report.dispositions, report.changed)
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

/// `(uuid, human_is_eligible)` of every record, as the app's hours read it.
fn eligibility(directory: &TempDir) -> Vec<(String, i64)> {
    Connection::open(directory.path().join("launch.sqlite"))
        .unwrap()
        .prepare("SELECT uuid,human_is_eligible FROM v_human_inputs ORDER BY uuid")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn open(directory: &TempDir) -> Store {
    Store::open(directory.path().join("launch.sqlite")).unwrap()
}

#[test]
fn a_launch_shows_its_exact_parent_replays_silently_and_feeds_the_existing_human_rule() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    let before = eligibility(&directory);
    assert_eq!(
        apply(&mut store, &[proof(CHILD)]),
        (vec![CreationDisposition::Recorded], 1)
    );
    assert_eq!(
        shown(&store),
        [(CHILD.to_owned(), codex(PARENT), ParentEvidence::AgentLaunch)]
    );
    assert_eq!(
        store.claude_launch_creation(CHILD).unwrap(),
        Some((proof(CHILD), false))
    );
    let (base, conflicted) = store.session_creation(CHILD).unwrap().unwrap();
    assert!(!conflicted);
    assert_eq!(base.evidence_kind, CreationEvidence::CodexClaudeLaunch);
    assert_eq!(base.witness, CreationWitness::CodexExecSessionIdLaunch);
    // The existing, approved rule: every user message of the new child is
    // taken as the agent's. Nothing else changes.
    let after = eligibility(&directory);
    let changed: Vec<&str> = before
        .iter()
        .zip(&after)
        .filter(|(b, a)| b != a)
        .map(|(b, _)| b.0.as_str())
        .collect();
    assert_eq!(changed, [first(CHILD), format!("{CHILD}-later")]);
    assert!(
        after
            .iter()
            .filter(|(uuid, _)| uuid.starts_with(CHILD))
            .all(|(_, e)| *e == 0)
    );
    // Replays, in one batch and after a reopen, change nothing.
    assert_eq!(
        apply(&mut store, &[proof(CHILD), proof(CHILD)]),
        (vec![CreationDisposition::AlreadyRecorded; 2], 0)
    );
    drop(store);
    let mut store = open(&directory);
    assert_eq!(
        apply(&mut store, &[proof(CHILD)]),
        (vec![CreationDisposition::AlreadyRecorded], 0)
    );
    assert_eq!(eligibility(&directory), after);
}

#[test]
fn the_store_refuses_what_it_cannot_tie_to_one_child_and_one_parent() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    let with = |edit: &dyn Fn(&mut ClaudeLaunchCreationProof)| {
        let mut proof = proof(CHILD);
        edit(&mut proof);
        proof
    };
    let abstained = |reason| CreationDisposition::Abstained(reason);
    for (proof, reason) in [
        (
            with(&|p| p.first_record_uuid = format!("{CHILD}-later")),
            CreationAbstention::FirstInputMismatch,
        ),
        (
            with(&|p| p.first_record_uuid = first(SECOND)),
            CreationAbstention::FirstInputMismatch,
        ),
        (
            with(&|p| p.parent_session_id = codex(OTHER_PARENT)),
            CreationAbstention::ParentMismatch,
        ),
        (
            with(&|p| p.parent_native_session_id = OTHER_PARENT.into()),
            CreationAbstention::ParentMismatch,
        ),
        (
            with(&|p| p.child_session_id = "missing".into()),
            CreationAbstention::MissingChild,
        ),
        (
            with(&|p| p.child_native_session_id = SECOND.into()),
            CreationAbstention::ChildIdentityMismatch,
        ),
        (
            with(&|p| p.parent_session_id = CHILD.into()),
            CreationAbstention::SelfLink,
        ),
    ] {
        assert_eq!(
            apply(&mut store, std::slice::from_ref(&proof)),
            (vec![abstained(reason)], 0),
            "{proof:?}"
        );
    }
    for bad in [
        with(&|p| p.evidence_version = 2),
        with(&|p| p.segment_rollout_id = "not-a-rollout".into()),
        with(&|p| p.segment_rollout_id = ROLLOUT.to_uppercase()),
        with(&|p| p.launch_ordinal = None),
        with(&|p| p.acknowledgment_ordinal = Some(1)),
        with(&|p| p.launch_call_id = "call with space".into()),
        with(&|p| p.process_session_id = Some(String::new())),
    ] {
        assert!(
            store
                .record_claude_launch_creations(std::slice::from_ref(&bad), T)
                .is_err(),
            "{bad:?}"
        );
    }
    // A flat history's proof has no ordinals, and a direct exit no handle.
    let flat = with(&|p| {
        p.launch_ordinal = None;
        p.acknowledgment_ordinal = None;
        p.process_session_id = None;
        p.acknowledgment_call_id = p.launch_call_id.clone();
    });
    assert_eq!(
        apply(&mut store, &[flat]),
        (vec![CreationDisposition::Recorded], 1)
    );
    assert!(shown(&store).iter().all(|(child, _, _)| child == CHILD));
}

/// Crossed claims A-X, A-Y, B-Y, across both Claude CLI kinds: one launch
/// claimed for two children withholds both, and a second anchor for a child
/// withholds it, whichever kind each claim is.
#[test]
fn crossed_claims_across_both_cli_kinds_withhold_every_contradiction() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    // A-X: launch A (an artifact proof) creates X.
    let report = store
        .record_cli_artifact_creations(&[artifact(CHILD, "call_a")], T)
        .unwrap();
    assert_eq!(report.dispositions, [CreationDisposition::Recorded]);
    // A-Y: the same launch, by the native kind, claims Y: both withheld.
    let mut a_y = proof(SECOND);
    a_y.launch_call_id = "call_a".into();
    assert_eq!(
        apply(&mut store, &[a_y]).0,
        [CreationDisposition::Conflicted]
    );
    assert!(
        store.session_creation(CHILD).unwrap().unwrap().1,
        "X withheld"
    );
    assert!(
        store.session_creation(SECOND).unwrap().unwrap().1,
        "Y withheld"
    );
    // B-Y: another launch for Y stays withheld.
    let mut b_y = proof(SECOND);
    b_y.launch_call_id = "call_b".into();
    assert_eq!(
        apply(&mut store, &[b_y]).0,
        [CreationDisposition::Conflicted]
    );
    assert!(shown(&store).is_empty());

    // A second anchor of the other kind for an accepted child conflicts it.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    assert_eq!(
        apply(&mut store, &[proof(CHILD)]).0,
        [CreationDisposition::Recorded]
    );
    let report = store
        .record_cli_artifact_creations(&[artifact(CHILD, "call_launch_c1")], T)
        .unwrap();
    assert_eq!(report.dispositions, [CreationDisposition::Conflicted]);
    assert!(store.session_creation(CHILD).unwrap().unwrap().1);
    // And a different launch of the same kind too, once, for good.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    apply(&mut store, &[proof(CHILD)]);
    let mut again = proof(CHILD);
    again.launch_call_id = "call_other".into();
    assert_eq!(
        apply(&mut store, &[again.clone()]),
        (vec![CreationDisposition::Conflicted], 1)
    );
    assert_eq!(
        apply(&mut store, &[again]),
        (vec![CreationDisposition::Conflicted], 0)
    );
    assert_eq!(
        apply(&mut store, &[proof(CHILD)]).0,
        [CreationDisposition::Conflicted]
    );
    // The table enforces the new kind's own shape.
    let connection = Connection::open(directory.path().join("launch.sqlite")).unwrap();
    for statement in [
        "UPDATE session_creation_relations SET segment_rollout_id='x'",
        "UPDATE session_creation_relations SET state='accepted'",
        "DELETE FROM session_creation_relations",
    ] {
        assert!(connection.execute(statement, []).is_err(), "{statement}");
    }
}

/// Every stored column of the one relation of `child`, as text.
fn relation_row(directory: &TempDir, child: &str) -> String {
    Connection::open(directory.path().join("launch.sqlite"))
        .unwrap()
        .query_row(
            "SELECT json_array(child_session_id,child_host,child_native_session_id,parent_host,
                 parent_native_session_id,evidence_kind,evidence_version,witness,state,
                 recorded_at,parent_session_id,first_record_uuid,launch_call_id,
                 launch_operation_index,process_session_id,completion_call_id,
                 output_read_call_id,provider_result_uuid,completion_operation_index,
                 segment_rollout_id,launch_ordinal,completion_ordinal)
             FROM session_creation_relations WHERE child_session_id=?1",
            [child],
            |row| row.get(0),
        )
        .unwrap()
}

/// A relation the previous launch version accepted, stored as it wrote it:
/// version 1, its completion poll as the anchor's second position, and the
/// launch claimed for the child.
fn insert_version_1(directory: &TempDir, proof: &ClaudeLaunchCreationProof) {
    let connection = Connection::open(directory.path().join("launch.sqlite")).unwrap();
    connection
        .execute(
            "INSERT INTO session_creation_relations(child_session_id,child_host,
                 child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
                 evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
                 launch_call_id,launch_operation_index,process_session_id,completion_call_id,
                 completion_operation_index,segment_rollout_id,launch_ordinal,completion_ordinal)
             VALUES (?1,'claude',?2,'codex',?3,'codex_claude_launch',1,
                 'codex_exec_session_id_launch','accepted',?4,?5,?6,?7,?8,'56751',
                 'call_old_completion',1,?9,?10,?11)",
            rusqlite::params![
                proof.child_session_id,
                proof.child_native_session_id,
                proof.parent_native_session_id,
                T - 1,
                proof.parent_session_id,
                proof.first_record_uuid,
                proof.launch_call_id,
                proof.launch_operation_index,
                proof.segment_rollout_id,
                proof.launch_ordinal,
                proof.launch_ordinal.map(|ordinal| ordinal + 900),
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cli_artifact_launch_owners VALUES (?1,?2,?3,?4,0)",
            rusqlite::params![
                proof.parent_session_id,
                proof.launch_call_id,
                proof.launch_operation_index,
                proof.child_session_id
            ],
        )
        .unwrap();
}

/// A proof of this version for the same creation anchor as a relation
/// already accepted — the same child, parent, first input and launch
/// operation (and, for an earlier launch relation, the same history file and
/// launch row) — replays it: the stored row stays exactly as it is, whatever
/// its acknowledgment, completion, handle or version. A different anchor
/// still withholds the child.
#[test]
fn the_same_creation_anchor_replays_an_accepted_relation_exactly() {
    let setup = || {
        let directory = TempDir::new().unwrap();
        let mut store = open(&directory);
        seed(&mut store);
        (directory, store)
    };
    // The previous version's relation for this launch.
    let (directory, mut store) = setup();
    insert_version_1(&directory, &proof(CHILD));
    let before = relation_row(&directory, CHILD);
    let mut acknowledged = proof(CHILD);
    acknowledged.process_session_id = None;
    acknowledged.acknowledgment_call_id = acknowledged.launch_call_id.clone();
    acknowledged.acknowledgment_ordinal = Some(3547);
    for replay in [acknowledged.clone(), proof(CHILD), acknowledged] {
        assert_eq!(
            apply(&mut store, &[replay]),
            (vec![CreationDisposition::AlreadyRecorded], 0)
        );
        assert_eq!(relation_row(&directory, CHILD), before);
    }
    assert_eq!(
        shown(&store),
        [(CHILD.to_owned(), codex(PARENT), ParentEvidence::AgentLaunch)]
    );
    // An accepted foreground CLI relation of the same launch and first input.
    let (directory, mut store) = setup();
    let mut same = artifact(CHILD, &proof(CHILD).launch_call_id);
    same.launch_operation_index = proof(CHILD).launch_operation_index;
    store.record_cli_artifact_creations(&[same], T).unwrap();
    let before = relation_row(&directory, CHILD);
    assert_eq!(
        apply(&mut store, &[proof(CHILD)]),
        (vec![CreationDisposition::AlreadyRecorded], 0)
    );
    assert_eq!(relation_row(&directory, CHILD), before);
    assert_eq!(
        shown(&store),
        [(CHILD.to_owned(), codex(PARENT), ParentEvidence::AgentLaunch)]
    );
    // Genuinely different anchors withhold the child.
    type Edit = fn(&mut ClaudeLaunchCreationProof);
    let edits: [(&str, Edit); 4] = [
        ("launch call", |p| p.launch_call_id = "call_other".into()),
        ("launch operation", |p| p.launch_operation_index = 1),
        ("history file", |p| {
            p.segment_rollout_id = "019a0000-0000-7000-8000-0000000000b9".into()
        }),
        ("launch row", |p| {
            p.launch_ordinal = Some(3000);
        }),
    ];
    for (name, edit) in edits {
        let (directory, mut store) = setup();
        insert_version_1(&directory, &proof(CHILD));
        let mut other = proof(CHILD);
        edit(&mut other);
        assert_eq!(
            apply(&mut store, &[other]),
            (vec![CreationDisposition::Conflicted], 1),
            "{name}"
        );
        assert!(store.session_creation(CHILD).unwrap().unwrap().1, "{name}");
    }
    let (_directory, mut store) = setup();
    store
        .record_cli_artifact_creations(&[artifact(CHILD, "call_other")], T)
        .unwrap();
    assert_eq!(
        apply(&mut store, &[proof(CHILD)]),
        (vec![CreationDisposition::Conflicted], 1)
    );
    assert!(shown(&store).is_empty());
}

/// An index at schema 14 — built by the migrations as shipped — holding
/// relations of both older kinds, a launch owner and a reviewer origin,
/// upgrades to 15 with every row unchanged, still immutable and still read.
#[test]
fn a_schema_14_index_upgrades_with_every_relation_unchanged() {
    const MIGRATIONS: [&str; 14] = [
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
    ];
    const VIEWS: [&str; 6] = [
        xt_store::views::RECORD_METADATA,
        xt_store::views::HUMAN_INPUTS,
        xt_store::views::RECORDS,
        xt_store::views::USAGE_RECORDS,
        xt_store::views::RESPONSE_USAGE,
        xt_store::views::SESSION_EVENTS,
    ];
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("launch.sqlite");
    {
        let connection = Connection::open(&path).unwrap();
        xt_store::timestamp::register_sqlite(&connection).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        connection
            .execute_batch("CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)")
            .unwrap();
        for (index, sql) in MIGRATIONS.iter().enumerate() {
            connection.execute_batch(sql).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_version VALUES (?1,'2026-09-29T00:00:00Z')",
                    [index as i64 + 1],
                )
                .unwrap();
        }
        // Today's views name the tables migrations 16 and 21 add; stand-ins
        // let them read this older index until the upgrade below replaces them.
        connection
            .execute_batch(
                "CREATE TABLE task_notification_inputs(record_uuid TEXT, session_id TEXT);
                 CREATE TABLE tool_sent_inputs(record_uuid TEXT, session_id TEXT);",
            )
            .unwrap();
        for view in VIEWS {
            connection.execute_batch(view).unwrap();
        }
        let exec = |sql: &str| connection.execute_batch(sql).unwrap();
        for (id, host, native) in [
            (codex(PARENT), "codex", PARENT),
            (
                "codex-019a0000-0000-7000-8000-0000000000bb".into(),
                "codex",
                "019a0000-0000-7000-8000-0000000000bb",
            ),
            (
                "codex-019a0000-0000-7000-8000-0000000000dd".into(),
                "codex",
                "019a0000-0000-7000-8000-0000000000dd",
            ),
            (CHILD.into(), "claude", CHILD),
        ] {
            connection
                .execute(
                    "INSERT INTO sessions(session_id,host,native_session_id,kind,source)
                     VALUES (?1,?2,?3,'user','readers_cli')",
                    [id, host.to_owned(), native.to_owned()],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO records(uuid,session_id,type,ts,ts_ms,role,is_meta,is_sidechain,
                     is_tool_result_carrier,text_len,is_human)
                 VALUES (?1,?2,'user','2026-09-29T07:58:40.536Z',1790668720536,'user',0,0,0,5,1)",
                [first(CHILD), CHILD.to_owned()],
            )
            .unwrap();
        exec(&format!(
            "INSERT INTO session_creation_relations(child_session_id,child_host,child_native_session_id,
                 parent_host,parent_native_session_id,evidence_kind,evidence_version,witness,state,recorded_at)
             VALUES ('codex-019a0000-0000-7000-8000-0000000000bb','codex','019a0000-0000-7000-8000-0000000000bb',
                 'codex','{PARENT}','codex_thread_spawn',1,'rollout_opening_session_meta','accepted',5);
             INSERT INTO session_creation_relations(child_session_id,child_host,child_native_session_id,
                 parent_host,parent_native_session_id,evidence_kind,evidence_version,witness,state,recorded_at,
                 parent_session_id,first_record_uuid,launch_call_id,launch_operation_index,process_session_id,
                 completion_call_id,output_read_call_id,provider_result_uuid)
             VALUES ('{CHILD}','claude','{CHILD}','codex','{PARENT}','cli_artifact_create',1,
                 'claude_cli_redirected_json_result','accepted',6,'codex-{PARENT}','{CHILD}-first','call_a',0,
                 '21036','call_c','call_r','0d000000-0000-4000-8000-000000000001');
             INSERT INTO cli_artifact_launch_owners VALUES ('codex-{PARENT}','call_a',0,'{CHILD}',0);
             INSERT INTO human_session_origins(session_id,host,native_session_id,parent_host,
                 parent_native_session_id,method,evidence_id,launch_id,rule_version)
             VALUES ('codex-019a0000-0000-7000-8000-0000000000dd','codex','019a0000-0000-7000-8000-0000000000dd',
                 'codex','{PARENT}','native_reviewer_header','rollout_opening_session_meta',
                 'native_reviewer_header',1);"
        ));
    }
    let dump = || -> Vec<String> {
        let connection = Connection::open(&path).unwrap();
        [
            "SELECT json_group_array(json_array(child_session_id,child_host,child_native_session_id,
                 parent_host,parent_native_session_id,evidence_kind,evidence_version,witness,state,
                 recorded_at,parent_session_id,first_record_uuid,launch_call_id,launch_operation_index,
                 process_session_id,completion_call_id,output_read_call_id,provider_result_uuid))
             FROM (SELECT * FROM session_creation_relations ORDER BY 1)",
            "SELECT json_group_array(json_array(parent_session_id,launch_call_id,launch_operation_index,
                 first_child_session_id,disputed)) FROM cli_artifact_launch_owners",
            "SELECT json_group_array(json_array(session_id,method,conflicted)) FROM human_session_origins",
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
        .execute_batch("DROP TABLE task_notification_inputs; DROP TABLE tool_sent_inputs")
        .unwrap();
    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 23);
        assert_eq!(dump(), before);
        let parents: Vec<(String, String, ParentEvidence)> = shown(&store);
        assert!(parents.contains(&(
            "codex-019a0000-0000-7000-8000-0000000000bb".into(),
            codex(PARENT),
            ParentEvidence::NativeSpawn
        )));
        assert!(parents.contains(&(CHILD.into(), codex(PARENT), ParentEvidence::AgentLaunch)));
        assert!(parents.contains(&(
            "codex-019a0000-0000-7000-8000-0000000000dd".into(),
            codex(PARENT),
            ParentEvidence::NativeReviewer
        )));
    }
    let connection = Connection::open(&path).unwrap();
    for statement in [
        "UPDATE session_creation_relations SET parent_native_session_id='x'",
        "DELETE FROM session_creation_relations",
        "UPDATE session_creation_relations SET completion_operation_index=1",
    ] {
        assert!(connection.execute(statement, []).is_err(), "{statement}");
    }
    // The old kinds cannot carry the new kind's columns.
    assert!(
        connection
            .execute(
                "UPDATE session_creation_relations SET state='conflicted',segment_rollout_id=?1",
                [ROLLOUT],
            )
            .is_err()
    );
}

mod group_contract {
    //! One published validation per thread: an attempt marks the thread
    //! pending, stages apart, and publishes at once only as the latest
    //! attempt from the revision it started from; a link is written only
    //! while everything it was checked against is unchanged.
    use super::*;
    use xt_store::claude_launch::{
        CandidateRow, ChildState, GroupStatus, LaunchCandidate, LaunchKey, MemberRecord,
        SegmentGeneration, SourceVerdict,
    };

    fn member(length: i64) -> MemberRecord {
        MemberRecord {
            rollout_id: ROLLOUT.into(),
            generation: SegmentGeneration {
                device: 1,
                inode: 2,
                length,
                mtime_ns: 3,
                ctime_ns: 4,
            },
            scanned_length: length,
            valid: true,
        }
    }

    fn candidate(child: &str, acknowledgment_offset: i64) -> LaunchCandidate {
        LaunchCandidate {
            key: LaunchKey {
                parent_native_session_id: PARENT.into(),
                rollout_id: ROLLOUT.into(),
                launch_call_id: format!("call_launch_{}", &child[child.len() - 2..]),
                launch_operation_index: 0,
            },
            child_native_session_id: child.into(),
            acknowledgment_call_id: "call_completion".into(),
            acknowledgment_operation_index: 0,
            process_session_id: Some("56751".into()),
            launch_offset: 100,
            acknowledgment_offset,
            launch_ordinal: Some(3546),
            acknowledgment_ordinal: Some(3939),
            binding_call_offset: None,
            binding_output_offset: None,
            child_host: xt_store::Host::Claude,
            launch_check_fingerprint: Some("0".repeat(64)),
        }
    }

    /// Publish one validation of `candidates`; returns its revision.
    fn publish(store: &mut Store, candidates: &[LaunchCandidate], length: i64) -> i64 {
        let allocation = store
            .begin_claude_launch_validation(PARENT, CLAUDE_LAUNCH_CREATE_VERSION)
            .unwrap();
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
                    &[member(length)],
                    candidates.len(),
                )
                .unwrap()
        );
        allocation.allocated
    }

    fn open_rows(store: &Store) -> Vec<CandidateRow> {
        store
            .claude_launch_open_candidates(PARENT, None, 100)
            .unwrap()
    }

    fn launch_proof(row: &CandidateRow) -> ClaudeLaunchCreationProof {
        let mut proof = proof(&row.candidate.child_native_session_id);
        proof.launch_call_id = row.candidate.key.launch_call_id.clone();
        proof
    }

    #[test]
    fn only_the_latest_attempt_publishes_and_at_once() {
        let directory = TempDir::new().unwrap();
        let mut store = open(&directory);
        seed(&mut store);
        let first = store
            .begin_claude_launch_validation(PARENT, CLAUDE_LAUNCH_CREATE_VERSION)
            .unwrap();
        assert!(
            store
                .stage_claude_launch_candidates(PARENT, first.allocated, &[candidate(CHILD, 900)])
                .unwrap()
        );
        // Staged launches are not launches yet.
        assert!(open_rows(&store).is_empty());
        let group = store.claude_launch_group(PARENT).unwrap().unwrap();
        assert_eq!((group.status, group.revision), (GroupStatus::Pending, None));
        // A second attempt supersedes the first: its staged rows are gone and
        // it can neither stage nor publish.
        let second = store
            .begin_claude_launch_validation(PARENT, CLAUDE_LAUNCH_CREATE_VERSION)
            .unwrap();
        assert!(second.allocated > first.allocated);
        assert!(
            !store
                .stage_claude_launch_candidates(PARENT, first.allocated, &[candidate(CHILD, 900)])
                .unwrap()
        );
        assert!(
            !store
                .publish_claude_launch_validation(
                    PARENT,
                    first,
                    GroupStatus::Valid,
                    &[member(1000)],
                    1
                )
                .unwrap()
        );
        // The latest publishes only with exactly what it staged.
        assert!(
            store
                .stage_claude_launch_candidates(PARENT, second.allocated, &[candidate(CHILD, 900)])
                .unwrap()
        );
        assert!(
            !store
                .publish_claude_launch_validation(
                    PARENT,
                    second,
                    GroupStatus::Valid,
                    &[member(1000)],
                    2
                )
                .unwrap()
        );
        assert!(
            !store
                .publish_claude_launch_validation(
                    PARENT,
                    second,
                    GroupStatus::Invalid,
                    &[member(1000)],
                    1
                )
                .unwrap()
        );
        assert!(
            store
                .publish_claude_launch_validation(
                    PARENT,
                    second,
                    GroupStatus::Valid,
                    &[member(1000)],
                    1
                )
                .unwrap()
        );
        let group = store.claude_launch_group(PARENT).unwrap().unwrap();
        assert_eq!(
            (group.status, group.revision),
            (GroupStatus::Valid, Some(second.allocated))
        );
        assert_eq!(open_rows(&store).len(), 1);
        assert_eq!(store.claude_launch_members(PARENT).unwrap(), [member(1000)]);
        assert_eq!(store.claude_launch_summary().unwrap().staged, 0);
        // A new attempt makes the thread pending before anything else.
        store
            .begin_claude_launch_validation(PARENT, CLAUDE_LAUNCH_CREATE_VERSION)
            .unwrap();
        assert!(
            open_rows(&store).is_empty(),
            "pending hides the published launches"
        );
        // Identifiers the tables cannot hold are refused before a statement.
        let mut bad = candidate(CHILD, 900);
        bad.key.launch_call_id = "x".repeat(257);
        assert!(
            store
                .stage_claude_launch_candidates(PARENT, 99, &[bad])
                .is_err()
        );
    }

    #[test]
    fn binding_line_offsets_stage_publish_and_reject_partial_or_reversed_pairs() {
        let directory = TempDir::new().unwrap();
        let mut store = open(&directory);
        seed(&mut store);
        let mut bound = candidate(CHILD, 900);
        bound.binding_call_offset = Some(10);
        bound.binding_output_offset = Some(50);
        publish(&mut store, &[bound.clone()], 1000);
        assert_eq!(open_rows(&store).remove(0).candidate, bound);
        for (call, output) in [
            (Some(10), None),
            (None, Some(50)),
            (Some(50), Some(10)),
            (Some(10), Some(100)),
        ] {
            let mut bad = bound.clone();
            bad.binding_call_offset = call;
            bad.binding_output_offset = output;
            assert!(
                store
                    .stage_claude_launch_candidates(PARENT, 999, &[bad])
                    .is_err()
            );
        }
    }

    #[test]
    fn the_guarded_write_refuses_anything_that_moved_and_linked_survives_only_unchanged() {
        let directory = TempDir::new().unwrap();
        let mut store = open(&directory);
        seed(&mut store);
        publish(&mut store, &[candidate(CHILD, 900)], 1000);
        let row = open_rows(&store).remove(0);
        let proof = launch_proof(&row);
        // Another generation of the member than the one checked.
        assert!(
            store
                .record_claude_launch_guarded(&proof, &row, &[member(999)], T)
                .unwrap()
                .is_none()
        );
        // Another structural field than the published one.
        let mut moved = row.clone();
        moved.candidate.acknowledgment_offset += 1;
        assert!(
            store
                .record_claude_launch_guarded(&proof, &moved, &[member(1000)], T)
                .unwrap()
                .is_none()
        );
        // A proof that is not its candidate's is refused outright.
        let mut other = proof.clone();
        other.acknowledgment_call_id = "call_other".into();
        assert!(
            store
                .record_claude_launch_guarded(&other, &row, &[member(1000)], T)
                .is_err()
        );
        // While a replacement validation runs.
        let pending = store
            .begin_claude_launch_validation(PARENT, CLAUDE_LAUNCH_CREATE_VERSION)
            .unwrap();
        assert!(
            store
                .record_claude_launch_guarded(&proof, &row, &[member(1000)], T)
                .unwrap()
                .is_none()
        );
        assert!(store.claude_launch_creation(CHILD).unwrap().is_none());
        // Once published again, identical, it links.
        assert!(
            store
                .stage_claude_launch_candidates(PARENT, pending.allocated, &[candidate(CHILD, 900)])
                .unwrap()
        );
        assert!(
            store
                .publish_claude_launch_validation(
                    PARENT,
                    pending,
                    GroupStatus::Valid,
                    &[member(1000)],
                    1
                )
                .unwrap()
        );
        let row = open_rows(&store).remove(0);
        let report = store
            .record_claude_launch_guarded(&proof, &row, &[member(1000)], T)
            .unwrap()
            .unwrap();
        assert_eq!(report.dispositions, [CreationDisposition::Recorded]);
        assert!(open_rows(&store).is_empty());
        // A new validation with the identical launch keeps it linked...
        publish(&mut store, &[candidate(CHILD, 900)], 1000);
        assert!(open_rows(&store).is_empty());
        assert_eq!(store.claude_launch_summary().unwrap().candidates_linked, 1);
        // ...a changed one is looked at again, and meets the conflict checks.
        publish(&mut store, &[candidate(CHILD, 901)], 1000);
        let row = open_rows(&store).remove(0);
        assert_eq!(row.child_state, ChildState::Waiting);
        let mut changed = launch_proof(&row);
        changed.acknowledgment_call_id = row.candidate.acknowledgment_call_id.clone();
        let report = store
            .record_claude_launch_guarded(&changed, &row, &[member(1000)], T)
            .unwrap()
            .unwrap();
        assert_eq!(report.dispositions, [CreationDisposition::AlreadyRecorded]);
    }

    #[test]
    fn a_child_import_reopens_only_child_side_rejections() {
        let directory = TempDir::new().unwrap();
        let mut store = open(&directory);
        seed(&mut store);
        publish(
            &mut store,
            &[candidate(CHILD, 900), candidate(SECOND, 900)],
            1000,
        );
        let rows = open_rows(&store);
        assert!(
            store
                .set_claude_launch_child_state(&rows[0], ChildState::Rejected)
                .unwrap()
        );
        assert!(store.reject_claude_launch_source(&rows[1]).unwrap());
        assert_eq!(store.reopen_claude_launch_child(CHILD).unwrap(), 1);
        assert_eq!(store.reopen_claude_launch_child(SECOND).unwrap(), 0);
        let open = open_rows(&store);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].candidate.child_native_session_id, CHILD);
        let second = store
            .claude_launch_candidates_for_child(SECOND, None, 10)
            .unwrap();
        assert_eq!(second[0].source_verdict, SourceVerdict::Rejected);
        // Pages follow key order with no row twice.
        let first_page = store
            .claude_launch_candidates_for_child(CHILD, None, 1)
            .unwrap();
        let next = store
            .claude_launch_candidates_for_child(CHILD, Some(&first_page[0].candidate.key), 1)
            .unwrap();
        assert!(next.is_empty());
    }
}
