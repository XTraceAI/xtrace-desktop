//! Foreground Claude CLI sub-sessions: a trusted validator's structural chain
//! is stored only after the store re-checks every identity it holds, an exact
//! replay is silent, any different claim on a child or on a launch withholds
//! what it contradicts, and nothing a person typed or any measurement changes.

use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    confirmation::{AutomatedInputProof, ConfirmationDisposition, EvidenceKind},
    creation::{
        CLI_ARTIFACT_CREATE_VERSION, CliArtifactCreationProof, CreationAbstention,
        CreationDisposition, CreationEvidence, CreationWitness, MAX_CREATION_PROOFS,
        SessionCreationProof,
    },
    session_list::{self, SessionFilter},
};

const PARENT: &str = "019a0000-0000-7000-8000-0000000000aa";
const OTHER_PARENT: &str = "019a0000-0000-7000-8000-0000000000ab";
const CHILD: &str = "0c000000-0000-4000-8000-0000000000c1";
const SECOND: &str = "0c000000-0000-4000-8000-0000000000c2";
const THIRD: &str = "0c000000-0000-4000-8000-0000000000c4";
const NATIVE_CHILD: &str = "019a0000-0000-7000-8000-0000000000ac";
const T: i64 = 1_788_782_400_000;

fn codex(native: &str) -> String {
    format!("codex-{native}")
}

fn first(child: &str) -> String {
    format!("{child}-first")
}

fn open(directory: &TempDir) -> Store {
    Store::open(directory.path().join("artifact.sqlite")).unwrap()
}

fn sql(directory: &TempDir) -> Connection {
    Connection::open(directory.path().join("artifact.sqlite")).unwrap()
}

fn record(value: serde_json::Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}

fn parent_session(store: &mut Store, native: &str) {
    let mut meta = SessionMeta::new(codex(native), "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some(native.into());
    meta.started_at_ms = Some(1);
    store.upsert_session(&meta, false).unwrap();
}

/// A Claude CLI session whose first input, agent work and every kind of
/// user-role record that is not an input all exist.
fn child_session(store: &mut Store, native: &str) {
    let mut meta = SessionMeta::new(native, "claude", SessionSource::Transcript);
    meta.native_session_id = Some(native.into());
    meta.surface = Some("sdk-cli".into());
    meta.started_at_ms = Some(2);
    store.upsert_session(&meta, false).unwrap();
    let text = |uuid: String, at: &str, extra: serde_json::Value| {
        let mut value = json!({"uuid":uuid,"type":"user","timestamp":at,
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic input"}]}});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        record(value)
    };
    store
        .upsert_records(
            native,
            &[
                text(
                    format!("{native}-meta"),
                    "2026-09-07T11:59:00Z",
                    json!({"isMeta":true}),
                ),
                text(
                    format!("{native}-side"),
                    "2026-09-07T11:59:30Z",
                    json!({"isSidechain":true}),
                ),
                text(first(native), "2026-09-07T12:00:00Z", json!({})),
                record(json!({"uuid":format!("{native}-agent"),"type":"assistant",
                    "timestamp":"2026-09-07T12:01:00Z",
                    "message":{"id":format!("{native}-msg"),"role":"assistant",
                        "model":"synthetic-model",
                        "content":[{"type":"tool_use","name":"Read","input":{"path":"synthetic"}}],
                        "usage":{"input_tokens":10,"output_tokens":5,
                            "cache_read_input_tokens":0,"cache_creation_input_tokens":0}}})),
                record(json!({"uuid":format!("{native}-result"),"type":"user",
                    "timestamp":"2026-09-07T12:02:00Z",
                    "message":{"role":"user","content":[{"type":"tool_result",
                        "tool_use_id":"t","content":"synthetic"}]}})),
                text(format!("{native}-later"), "2026-09-07T12:10:00Z", json!({})),
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

fn proof(child: &str) -> CliArtifactCreationProof {
    CliArtifactCreationProof {
        child_session_id: child.into(),
        child_native_session_id: child.into(),
        parent_session_id: codex(PARENT),
        parent_native_session_id: PARENT.into(),
        first_record_uuid: first(child),
        launch_call_id: format!("call_launch_{}", &child[child.len() - 2..]),
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
    proofs: &[CliArtifactCreationProof],
) -> (Vec<CreationDisposition>, usize) {
    let report = store.record_cli_artifact_creations(proofs, T).unwrap();
    (report.dispositions, report.changed)
}

fn abstained(reason: CreationAbstention) -> CreationDisposition {
    CreationDisposition::Abstained(reason)
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

/// Every row of every table the relation must never touch, as JSON text.
fn untouched(connection: &Connection) -> Vec<String> {
    [
        "SELECT json_group_array(json_array(session_id,host,native_session_id,kind,title,surface,
             started_at_ms,first_ts,last_ts,source)) FROM (SELECT * FROM sessions ORDER BY 1)",
        "SELECT json_group_array(json_array(uuid,session_id,type,ts_ms,is_meta,is_sidechain,role,
             is_tool_result_carrier,text_len,is_human,is_command,content_json))
         FROM (SELECT * FROM records ORDER BY 1)",
        "SELECT json_group_array(json_array(uuid,is_human,raw_is_human,confirmed_automated_input))
         FROM (SELECT * FROM v_records ORDER BY uuid)",
        "SELECT json_group_array(json_array(uuid,input_tokens,output_tokens))
         FROM (SELECT * FROM usage ORDER BY 1)",
        "SELECT CAST(count(*) AS TEXT) FROM confirmed_automated_inputs",
    ]
    .into_iter()
    .map(|query| connection.query_row(query, [], |row| row.get(0)).unwrap())
    .collect()
}

#[test]
fn an_exact_launch_shows_its_parent_and_replays_silently() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    let before = untouched(&sql(&directory));
    let pages_before = pages(&store);

    assert_eq!(
        apply(&mut store, &[proof(CHILD)]),
        (vec![CreationDisposition::Recorded], 1)
    );
    assert_eq!(shown(&store), [(CHILD.to_owned(), codex(PARENT))]);
    let row = store
        .sessions_page_filtered(&SessionFilter::default(), None)
        .unwrap()
        .into_iter()
        .find(|row| row.id == CHILD)
        .unwrap();
    let parent = row.parent.unwrap();
    assert_eq!(parent.host, "codex");
    assert_eq!(parent.evidence, CreationEvidence::CliArtifactCreate.into());
    assert_eq!(
        store.cli_artifact_creation(CHILD).unwrap(),
        Some((proof(CHILD), false))
    );
    let (base, conflicted) = store.session_creation(CHILD).unwrap().unwrap();
    assert!(!conflicted);
    assert_eq!(base.witness, CreationWitness::ClaudeCliRedirectedJsonResult);
    assert_eq!(
        (base.child_host, base.parent_host),
        (Host::Claude, Host::Codex)
    );

    // Nothing but the relation changed: sessions, records, human
    // classification, usage and confirmations are byte-identical, and every
    // page holds the same rows in the same order.
    assert_eq!(untouched(&sql(&directory)), before);
    assert_eq!(pages(&store), pages_before);

    // Replay in one batch and across a reopen changes nothing and reports no
    // change.
    assert_eq!(
        apply(&mut store, &[proof(CHILD), proof(CHILD)]),
        (
            vec![
                CreationDisposition::AlreadyRecorded,
                CreationDisposition::AlreadyRecorded
            ],
            0
        )
    );
    drop(store);
    let mut store = open(&directory);
    assert_eq!(
        apply(&mut store, &[proof(CHILD)]),
        (vec![CreationDisposition::AlreadyRecorded], 0)
    );
    assert_eq!(shown(&store), [(CHILD.to_owned(), codex(PARENT))]);
    assert_eq!(untouched(&sql(&directory)), before);
}

/// Stage A's confirmation of a dispatched input stays exactly its own: a
/// creation relation neither needs it nor makes one, and a confirmation
/// applied afterwards is judged by its own rules alone.
#[test]
fn a_creation_relation_never_confirms_or_classifies_an_input() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    apply(&mut store, &[proof(CHILD)]);
    let connection = sql(&directory);
    let human: Option<i64> = connection
        .query_row(
            "SELECT is_human FROM v_records WHERE uuid=?1",
            [first(CHILD)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(human, Some(1), "the first input stays human-classified");
    assert!(
        store
            .automated_input_confirmations(CHILD)
            .unwrap()
            .is_empty()
    );
    let confirmation = AutomatedInputProof {
        record_uuid: first(CHILD),
        session_id: CHILD.into(),
        native_session_id: CHILD.into(),
        parent_host: Host::Codex,
        parent_session_id: codex(PARENT),
        parent_tool_call_id: proof(CHILD).launch_call_id,
        parent_operation_index: 0,
        parent_result_id: None,
        evidence_kind: EvidenceKind::AgentDispatch,
        matcher_version: 1,
    };
    assert_eq!(
        store
            .apply_automated_input_confirmations(&[confirmation], T)
            .unwrap()
            .dispositions,
        [ConfirmationDisposition::Confirmed]
    );
    assert_eq!(
        store.cli_artifact_creation(CHILD).unwrap(),
        Some((proof(CHILD), false))
    );
}

#[test]
fn every_identity_the_index_holds_is_checked_before_anything_is_stored() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    // A Claude session that shares CHILD's native identity makes it ambiguous
    // only once it exists; a judge session is never a user session.
    let mut judge = SessionMeta::new("judge-claude", "claude", SessionSource::Transcript);
    judge.native_session_id = Some("judge-claude".into());
    store.upsert_session(&judge, false).unwrap();
    sql(&directory)
        .execute(
            "UPDATE sessions SET kind='judge' WHERE session_id='judge-claude'",
            [],
        )
        .unwrap();

    let with = |change: &dyn Fn(&mut CliArtifactCreationProof)| {
        let mut proof = proof(CHILD);
        change(&mut proof);
        proof
    };
    let cases = [
        (
            with(&|p| p.child_session_id = "missing".into()),
            CreationAbstention::MissingChild,
        ),
        (
            with(&|p| {
                p.child_session_id = "judge-claude".into();
                p.child_native_session_id = "judge-claude".into();
            }),
            CreationAbstention::NotUserSession,
        ),
        (
            with(&|p| p.child_native_session_id = SECOND.into()),
            CreationAbstention::ChildIdentityMismatch,
        ),
        (
            with(&|p| p.child_session_id = codex(OTHER_PARENT)),
            CreationAbstention::ChildIdentityMismatch,
        ),
        // The first input of another child, a later input, a meta or
        // sidechain record, a tool result, agent work, or no record at all.
        (
            with(&|p| p.first_record_uuid = first(SECOND)),
            CreationAbstention::FirstInputMismatch,
        ),
        (
            with(&|p| p.first_record_uuid = format!("{CHILD}-later")),
            CreationAbstention::FirstInputMismatch,
        ),
        (
            with(&|p| p.first_record_uuid = format!("{CHILD}-meta")),
            CreationAbstention::FirstInputMismatch,
        ),
        (
            with(&|p| p.first_record_uuid = format!("{CHILD}-side")),
            CreationAbstention::FirstInputMismatch,
        ),
        (
            with(&|p| p.first_record_uuid = format!("{CHILD}-result")),
            CreationAbstention::FirstInputMismatch,
        ),
        (
            with(&|p| p.first_record_uuid = format!("{CHILD}-agent")),
            CreationAbstention::FirstInputMismatch,
        ),
        (
            with(&|p| p.first_record_uuid = "absent".into()),
            CreationAbstention::FirstInputMismatch,
        ),
        // A parent that is not indexed, names another native identity, is
        // a Claude session or is the child itself.
        (
            with(&|p| p.parent_session_id = codex("unindexed")),
            CreationAbstention::ParentMismatch,
        ),
        (
            with(&|p| p.parent_native_session_id = OTHER_PARENT.into()),
            CreationAbstention::ParentMismatch,
        ),
        (
            with(&|p| {
                p.parent_session_id = SECOND.into();
                p.parent_native_session_id = SECOND.into();
            }),
            CreationAbstention::ParentMismatch,
        ),
        (
            with(&|p| {
                p.parent_session_id = CHILD.into();
                p.parent_native_session_id = CHILD.into();
            }),
            CreationAbstention::SelfLink,
        ),
    ];
    for (bad, reason) in &cases {
        assert_eq!(
            apply(&mut store, std::slice::from_ref(bad)),
            (vec![abstained(*reason)], 0),
            "{reason:?}"
        );
    }
    assert!(store.cli_artifact_creation(CHILD).unwrap().is_none());

    // An earlier eligible input without a time leaves "first" unknown.
    let untimed = "0c000000-0000-4000-8000-0000000000c3";
    child_session(&mut store, untimed);
    store
        .upsert_records(
            untimed,
            &[record(json!({"uuid":"untimed-input","type":"user",
                "message":{"role":"user","content":[{"type":"text","text":"Synthetic"}]}}))],
            false,
        )
        .unwrap();
    assert_eq!(
        apply(&mut store, &[proof(untimed)]),
        (vec![abstained(CreationAbstention::FirstInputMismatch)], 0)
    );

    // Two indexed Codex user sessions with the parent's native identity.
    let mut twin = SessionMeta::new("codex-twin", "codex", SessionSource::ReadersCli);
    twin.native_session_id = Some(PARENT.into());
    store.upsert_session(&twin, false).unwrap();
    assert_eq!(
        apply(&mut store, &[proof(CHILD)]),
        (vec![abstained(CreationAbstention::AmbiguousParent)], 0)
    );
    // Two indexed Claude user sessions with the child's native identity.
    let mut copy = SessionMeta::new("claude-copy", "claude", SessionSource::Transcript);
    copy.native_session_id = Some(SECOND.into());
    store.upsert_session(&copy, false).unwrap();
    let mut second = proof(SECOND);
    second.parent_session_id = codex(OTHER_PARENT);
    second.parent_native_session_id = OTHER_PARENT.into();
    assert_eq!(
        apply(&mut store, &[second]),
        (vec![abstained(CreationAbstention::AmbiguousChild)], 0)
    );
    assert!(shown(&store).is_empty());
}

/// A second creation anchor for a child is a contradiction even when it names
/// the same parent: the child is withheld, the first witness is kept, and
/// neither the first proof nor the second is ever accepted again.
#[test]
fn a_different_proof_for_one_child_withholds_it_for_good() {
    let variants: [&dyn Fn(&mut CliArtifactCreationProof); 7] = [
        &|p| p.launch_call_id = "call_launch_again".into(),
        &|p| p.launch_operation_index = 1,
        &|p| p.process_session_id = "78787".into(),
        &|p| p.completion_call_id = "call_completion_again".into(),
        &|p| p.output_read_call_id = "call_read_again".into(),
        &|p| p.provider_result_uuid = "0d000000-0000-4000-8000-000000000002".into(),
        &|p| {
            p.parent_session_id = codex(OTHER_PARENT);
            p.parent_native_session_id = OTHER_PARENT.into();
        },
    ];
    for (index, change) in variants.into_iter().enumerate() {
        let directory = TempDir::new().unwrap();
        let mut store = open(&directory);
        seed(&mut store);
        apply(&mut store, &[proof(CHILD)]);
        let mut other = proof(CHILD);
        change(&mut other);
        assert_eq!(
            apply(&mut store, std::slice::from_ref(&other)),
            (vec![CreationDisposition::Conflicted], 1),
            "variant {index}"
        );
        assert!(shown(&store).is_empty(), "variant {index}");
        assert_eq!(
            store.cli_artifact_creation(CHILD).unwrap(),
            Some((proof(CHILD), true)),
            "the first witness stays, withheld"
        );
        drop(store);
        let mut store = open(&directory);
        assert_eq!(
            apply(&mut store, &[proof(CHILD), other]),
            (
                vec![
                    CreationDisposition::Conflicted,
                    CreationDisposition::Conflicted
                ],
                0
            ),
            "variant {index}: no replay is accepted again, and none is a change"
        );
        assert!(shown(&store).is_empty());
    }

    // The same contradiction inside one call ends the same way.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    let mut other = proof(CHILD);
    other.process_session_id = "78787".into();
    assert_eq!(
        apply(&mut store, &[proof(CHILD), other]).0,
        [
            CreationDisposition::Recorded,
            CreationDisposition::Conflicted
        ]
    );
    assert!(shown(&store).is_empty());

    // A proof whose first record is not the child's own abstains without
    // touching the accepted relation.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    apply(&mut store, &[proof(CHILD)]);
    let mut stray = proof(CHILD);
    stray.first_record_uuid = first(SECOND);
    assert_eq!(
        apply(&mut store, &[stray]),
        (vec![abstained(CreationAbstention::FirstInputMismatch)], 0)
    );
    assert_eq!(shown(&store), [(CHILD.to_owned(), codex(PARENT))]);
}

/// One launch operation created one child. A second child claimed for it
/// withholds both, including the one accepted first, and the second claim is
/// stored so that neither can later be accepted alone.
#[test]
fn one_launch_claimed_for_two_children_withholds_both() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    apply(&mut store, &[proof(CHILD)]);
    let mut stolen = proof(SECOND);
    stolen.launch_call_id = proof(CHILD).launch_call_id;
    assert_eq!(
        apply(&mut store, std::slice::from_ref(&stolen)),
        (vec![CreationDisposition::Conflicted], 1)
    );
    assert!(shown(&store).is_empty());
    assert_eq!(
        store.cli_artifact_creation(CHILD).unwrap(),
        Some((proof(CHILD), true))
    );
    assert_eq!(
        store.cli_artifact_creation(SECOND).unwrap(),
        Some((stolen.clone(), true))
    );
    drop(store);
    let mut store = open(&directory);
    assert_eq!(
        apply(&mut store, &[proof(CHILD), stolen.clone()]),
        (
            vec![
                CreationDisposition::Conflicted,
                CreationDisposition::Conflicted
            ],
            0
        )
    );
    assert!(shown(&store).is_empty());

    // A child that already has its own accepted launch, claimed again through
    // another child's launch: its own claim and the other child's are both
    // contradicted, and both are withheld.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    apply(&mut store, &[proof(CHILD), proof(SECOND)]);
    assert_eq!(shown(&store).len(), 2);
    let mut crossed = proof(SECOND);
    crossed.launch_call_id = proof(CHILD).launch_call_id;
    assert_eq!(
        apply(&mut store, &[crossed]),
        (vec![CreationDisposition::Conflicted], 1)
    );
    assert!(shown(&store).is_empty());
    assert_eq!(
        store.cli_artifact_creation(CHILD).unwrap(),
        Some((proof(CHILD), true))
    );
    assert_eq!(
        store.cli_artifact_creation(SECOND).unwrap(),
        Some((proof(SECOND), true))
    );

    // The same launch at another operation index, or another parent's call
    // with the same identifier, is a different launch.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    apply(&mut store, &[proof(CHILD)]);
    let mut next = stolen.clone();
    next.launch_operation_index = 1;
    assert_eq!(
        apply(&mut store, &[next]).0,
        [CreationDisposition::Recorded]
    );
    assert_eq!(shown(&store).len(), 2);

    // Both claims in one call: the second withholds the first.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    assert_eq!(
        apply(&mut store, &[proof(CHILD), stolen]),
        (
            vec![
                CreationDisposition::Recorded,
                CreationDisposition::Conflicted
            ],
            2
        )
    );
    assert!(shown(&store).is_empty());
}

/// `proof(child)` launched by another child's launch call.
fn launched_by(child: &str, launch: &str) -> CliArtifactCreationProof {
    let mut proof = proof(child);
    proof.launch_call_id = proof_launch(launch);
    proof
}

fn proof_launch(child: &str) -> String {
    proof(child).launch_call_id
}

/// `(launch call, first child, disputed)` for every claimed launch of PARENT.
fn owners(directory: &TempDir) -> Vec<(String, String, bool)> {
    sql(directory)
        .prepare(
            "SELECT launch_call_id,first_child_session_id,disputed FROM cli_artifact_launch_owners
             ORDER BY parent_session_id,launch_call_id,launch_operation_index",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// A native Codex spawn beside the CLI relations, which none of their
/// conflicts may touch.
fn native_spawn(store: &mut Store) {
    let mut meta = SessionMeta::new(codex(NATIVE_CHILD), "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some(NATIVE_CHILD.into());
    meta.started_at_ms = Some(3);
    store.upsert_session(&meta, false).unwrap();
    let spawn = SessionCreationProof {
        child_session_id: codex(NATIVE_CHILD),
        child_host: Host::Codex,
        child_native_session_id: NATIVE_CHILD.into(),
        parent_host: Host::Codex,
        parent_native_session_id: PARENT.into(),
        evidence_kind: CreationEvidence::CodexThreadSpawn,
        evidence_version: 1,
        witness: CreationWitness::RolloutOpeningSessionMeta,
    };
    assert_eq!(
        store
            .record_session_creations(&[spawn], T)
            .unwrap()
            .dispositions,
        [CreationDisposition::Recorded]
    );
}

/// A launch a child's stored relation never kept is still that child's: when
/// another child claims it, the launch is disputed and the other child is
/// withheld too, however the claims are ordered or split across calls.
#[test]
fn a_launch_claimed_for_a_second_child_is_disputed_for_good() {
    let (x, y) = (CHILD, SECOND);
    let native = |store: &Store| {
        shown(store)
            .into_iter()
            .filter(|(child, _)| child == &codex(NATIVE_CHILD))
            .count()
    };

    // Across calls: A on X, A again on Y, then B on Y.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    native_spawn(&mut store);
    assert_eq!(
        apply(&mut store, &[launched_by(CHILD, x)]),
        (vec![CreationDisposition::Recorded], 1)
    );
    assert_eq!(
        apply(&mut store, &[launched_by(CHILD, y)]),
        (vec![CreationDisposition::Conflicted], 1)
    );
    assert_eq!(
        apply(&mut store, &[launched_by(SECOND, y)]),
        (vec![CreationDisposition::Conflicted], 0),
        "the claim on a disputed launch is stored withheld"
    );
    assert_eq!(shown(&store), [(codex(NATIVE_CHILD), codex(PARENT))]);
    assert_eq!(
        store.cli_artifact_creation(SECOND).unwrap(),
        Some((launched_by(SECOND, y), true))
    );
    assert_eq!(
        owners(&directory),
        [
            (proof_launch(x), CHILD.to_owned(), false),
            (proof_launch(y), CHILD.to_owned(), true),
        ]
    );
    // Reopened, no replay in any order is accepted again, and none changes
    // anything.
    drop(store);
    let mut store = open(&directory);
    for batch in [
        vec![
            launched_by(SECOND, y),
            launched_by(CHILD, x),
            launched_by(CHILD, y),
        ],
        vec![launched_by(CHILD, x)],
        vec![launched_by(SECOND, y)],
    ] {
        let (dispositions, changed) = apply(&mut store, &batch);
        assert!(
            dispositions
                .iter()
                .all(|d| *d == CreationDisposition::Conflicted)
        );
        assert_eq!(changed, 0);
    }
    assert_eq!(shown(&store), [(codex(NATIVE_CHILD), codex(PARENT))]);
    assert_eq!(native(&store), 1);

    // The same three claims in one call end the same way.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    native_spawn(&mut store);
    assert_eq!(
        apply(
            &mut store,
            &[
                launched_by(CHILD, x),
                launched_by(CHILD, y),
                launched_by(SECOND, y),
            ]
        ),
        (
            vec![
                CreationDisposition::Recorded,
                CreationDisposition::Conflicted,
                CreationDisposition::Conflicted,
            ],
            2
        )
    );
    assert_eq!(shown(&store), [(codex(NATIVE_CHILD), codex(PARENT))]);
    drop(store);
    let mut store = open(&directory);
    assert_eq!(
        apply(&mut store, &[launched_by(SECOND, y)]),
        (vec![CreationDisposition::Conflicted], 0)
    );
    assert_eq!(native(&store), 1);

    // B accepted on Y first; A's second anchor on Y then withholds B too.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    assert_eq!(
        apply(&mut store, &[launched_by(SECOND, y), launched_by(CHILD, x)]),
        (
            vec![CreationDisposition::Recorded, CreationDisposition::Recorded],
            2
        )
    );
    assert_eq!(
        apply(&mut store, &[launched_by(CHILD, y)]),
        (vec![CreationDisposition::Conflicted], 1)
    );
    assert!(shown(&store).is_empty());
    assert_eq!(
        store.cli_artifact_creation(SECOND).unwrap(),
        Some((launched_by(SECOND, y), true))
    );
}

/// A child claimed through several launches owns each of them: any of them
/// claimed for another child is disputed, and an unrelated child's own
/// launch, or the same call identifier of another parent or operation, is
/// not.
#[test]
fn every_launch_a_child_was_claimed_through_stays_its_own() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    child_session(&mut store, THIRD);
    native_spawn(&mut store);
    let launches = ["call_x", "call_y", "call_z"];
    for (index, launch) in launches.iter().enumerate() {
        let mut claim = proof(CHILD);
        claim.launch_call_id = (*launch).into();
        let expected = if index == 0 {
            (vec![CreationDisposition::Recorded], 1)
        } else {
            (
                vec![CreationDisposition::Conflicted],
                usize::from(index == 1),
            )
        };
        assert_eq!(apply(&mut store, &[claim]), expected, "{launch}");
    }
    assert_eq!(
        apply(&mut store, &[proof(THIRD)]).0,
        [CreationDisposition::Recorded]
    );
    // The same call identifier at another operation, or under another parent,
    // is another launch.
    let mut elsewhere = proof(SECOND);
    elsewhere.launch_call_id = "call_z".into();
    elsewhere.launch_operation_index = 1;
    assert_eq!(
        apply(&mut store, &[elsewhere]).0,
        [CreationDisposition::Recorded]
    );
    drop(store);
    let mut store = open(&directory);
    let mut disputed = proof(SECOND);
    disputed.launch_call_id = "call_z".into();
    assert_eq!(
        apply(&mut store, &[disputed.clone()]),
        (vec![CreationDisposition::Conflicted], 1),
        "SECOND's accepted relation is contradicted by its claim on CHILD's launch"
    );
    let mut shown_now = shown(&store);
    shown_now.sort();
    let mut expected = vec![
        (THIRD.to_owned(), codex(PARENT)),
        (codex(NATIVE_CHILD), codex(PARENT)),
    ];
    expected.sort();
    assert_eq!(shown_now, expected);
    assert_eq!(
        owners(&directory)
            .into_iter()
            .filter(|(_, _, disputed)| *disputed)
            .collect::<Vec<_>>(),
        [("call_z".to_owned(), CHILD.to_owned(), true)]
    );

    // Another parent's call with the same identifier is another launch.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    apply(&mut store, &[proof(CHILD)]);
    let mut other_parent = launched_by(SECOND, CHILD);
    other_parent.parent_session_id = codex(OTHER_PARENT);
    other_parent.parent_native_session_id = OTHER_PARENT.into();
    assert_eq!(
        apply(&mut store, &[other_parent]).0,
        [CreationDisposition::Recorded]
    );
    assert_eq!(shown(&store).len(), 2);

    // A proof that abstains claims nothing.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    let mut stray = launched_by(SECOND, CHILD);
    stray.first_record_uuid = first(CHILD);
    assert_eq!(
        apply(&mut store, &[stray]).0,
        [abstained(CreationAbstention::FirstInputMismatch)]
    );
    assert!(owners(&directory).is_empty());
    assert_eq!(
        apply(&mut store, &[proof(CHILD)]).0,
        [CreationDisposition::Recorded]
    );
}

/// A claimed first input must carry text, and must still be the first user
/// input the child saved: an earlier input without text, an image alone,
/// came first.
#[test]
fn a_first_input_carries_text_and_nothing_was_saved_before_it() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    let input = |uuid: &str, at: &str, content: serde_json::Value| {
        record(json!({"uuid":uuid,"type":"user","timestamp":at,
            "message":{"role":"user","content":content}}))
    };
    let image =
        json!([{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AA=="}}]);
    let text = json!([{"type":"text","text":"Synthetic input"}]);
    let child = |store: &mut Store, native: &str, records: &[CanonicalRecord]| {
        let mut meta = SessionMeta::new(native, "claude", SessionSource::Transcript);
        meta.native_session_id = Some(native.into());
        meta.started_at_ms = Some(2);
        store.upsert_session(&meta, false).unwrap();
        store.upsert_records(native, records, false).unwrap();
    };
    let cases = [
        (
            "0c000000-0000-4000-8000-0000000000e1",
            vec![("first", json!([]))],
        ),
        (
            "0c000000-0000-4000-8000-0000000000e2",
            vec![("first", image.clone())],
        ),
        (
            "0c000000-0000-4000-8000-0000000000e3",
            vec![("earlier", image.clone()), ("first", text.clone())],
        ),
        (
            "0c000000-0000-4000-8000-0000000000e4",
            vec![("earlier", json!([])), ("first", text.clone())],
        ),
    ];
    for (native, records) in &cases {
        let records: Vec<_> = records
            .iter()
            .enumerate()
            .map(|(index, (name, content))| {
                let uuid = if *name == "first" {
                    first(native)
                } else {
                    format!("{native}-{name}")
                };
                input(
                    &uuid,
                    &format!("2026-09-07T12:0{index}:00Z"),
                    content.clone(),
                )
            })
            .collect();
        child(&mut store, native, &records);
        assert_eq!(
            apply(&mut store, &[proof(native)]),
            (vec![abstained(CreationAbstention::FirstInputMismatch)], 0),
            "{native}"
        );
    }
    // An input without text saved after the first is no obstacle.
    let later = "0c000000-0000-4000-8000-0000000000e5";
    child(
        &mut store,
        later,
        &[
            input(&first(later), "2026-09-07T12:00:00Z", text),
            input(&format!("{later}-image"), "2026-09-07T12:01:00Z", image),
        ],
    );
    assert_eq!(
        apply(&mut store, &[proof(later)]).0,
        [CreationDisposition::Recorded]
    );
    assert_eq!(shown(&store), [(later.to_owned(), codex(PARENT))]);
    assert_eq!(
        owners(&directory).len(),
        1,
        "only the valid proof claimed its launch"
    );
}

#[test]
fn malformed_proofs_or_the_wrong_path_reject_the_whole_call() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    let with = |change: &dyn Fn(&mut CliArtifactCreationProof)| {
        let mut proof = proof(CHILD);
        change(&mut proof);
        proof
    };
    let bad = [
        with(&|p| p.launch_call_id = String::new()),
        with(&|p| p.launch_call_id = "call with space".into()),
        with(&|p| p.process_session_id = "21036\n".into()),
        with(&|p| p.completion_call_id = "call_é".into()),
        with(&|p| p.output_read_call_id = "r".repeat(257)),
        with(&|p| p.provider_result_uuid = " ".into()),
        with(&|p| p.provider_result_uuid = String::new()),
        with(&|p| p.first_record_uuid = "\u{7}".into()),
        with(&|p| p.evidence_version = 0),
        with(&|p| p.evidence_version = CLI_ARTIFACT_CREATE_VERSION + 1),
    ];
    for bad in bad {
        assert!(
            store
                .record_cli_artifact_creations(&[proof(CHILD), bad.clone()], T)
                .is_err(),
            "{bad:?}"
        );
    }
    let many = vec![proof(CHILD); MAX_CREATION_PROOFS + 1];
    assert!(store.record_cli_artifact_creations(&many, T).is_err());
    // The native path cannot record this kind, with or without its witness.
    for witness in [
        CreationWitness::ClaudeCliRedirectedJsonResult,
        CreationWitness::RolloutOpeningSessionMeta,
    ] {
        let native = SessionCreationProof {
            child_session_id: CHILD.into(),
            child_host: Host::Claude,
            child_native_session_id: CHILD.into(),
            parent_host: Host::Codex,
            parent_native_session_id: PARENT.into(),
            evidence_kind: CreationEvidence::CliArtifactCreate,
            evidence_version: CLI_ARTIFACT_CREATE_VERSION,
            witness,
        };
        assert!(store.record_session_creations(&[native], T).is_err());
    }
    assert!(store.session_creation(CHILD).unwrap().is_none());
}

#[test]
fn the_table_holds_only_structural_tokens_and_each_kind_its_own_shape() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    apply(&mut store, &[proof(CHILD)]);
    let connection = sql(&directory);
    let columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('session_creation_relations') ORDER BY cid")
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
            "parent_host",
            "parent_native_session_id",
            "evidence_kind",
            "evidence_version",
            "witness",
            "state",
            "recorded_at",
            "parent_session_id",
            "first_record_uuid",
            "launch_call_id",
            "launch_operation_index",
            "process_session_id",
            "completion_call_id",
            "output_read_call_id",
            "provider_result_uuid",
            "completion_operation_index",
            "segment_rollout_id",
            "launch_ordinal",
            "completion_ordinal",
        ]
    );
    for forbidden in [
        "prompt", "command", "cmd", "path", "output", "body", "hash", "digest", "title", "role",
        "task", "text", "content", "json", "time_ms",
    ] {
        assert!(
            !columns
                .iter()
                .any(|column| column.contains(forbidden) && column != "output_read_call_id"),
            "{forbidden}"
        );
    }

    let insert = |kind: &str, witness: &str, child: &str, fields: [Option<&str>; 8]| {
        connection.execute(
            "INSERT INTO session_creation_relations(child_session_id,child_host,
                 child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
                 evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
                 launch_call_id,launch_operation_index,process_session_id,completion_call_id,
                 output_read_call_id,provider_result_uuid)
             VALUES (?1,?2,?1,'codex',?3,?4,1,?5,'accepted',0,?6,?7,?8,?9,?10,?11,?12,?13)",
            rusqlite::params![
                child,
                if kind == "codex_thread_spawn" {
                    "codex"
                } else {
                    "claude"
                },
                PARENT,
                kind,
                witness,
                fields[0],
                fields[1],
                fields[2],
                fields[3].map(|index| index.parse::<i64>().unwrap()),
                fields[4],
                fields[5],
                fields[6],
                fields[7],
            ],
        )
    };
    let full = [
        Some(codex(PARENT)),
        Some(first(SECOND)),
        Some("call".to_owned()),
        Some("0".to_owned()),
        Some("1".to_owned()),
        Some("done".to_owned()),
        Some("read".to_owned()),
        Some("result".to_owned()),
    ];
    let fields = |change: &dyn Fn(&mut [Option<String>; 8])| {
        let mut fields = full.clone();
        change(&mut fields);
        fields
    };
    fn as_refs(fields: &[Option<String>; 8]) -> [Option<&str>; 8] {
        fields.each_ref().map(Option::as_deref)
    }
    let artifact = "cli_artifact_create";
    let result = "claude_cli_redirected_json_result";
    // Each required field, a spaced or control token, the other kind's
    // witness, a record of another session, or a native row carrying any
    // artifact field.
    for index in 0..8 {
        let missing = fields(&|f| f[index] = None);
        assert!(
            insert(artifact, result, SECOND, as_refs(&missing)).is_err(),
            "{index}"
        );
    }
    for bad in ["a b", "a\tb", "é"] {
        let spaced = fields(&|f| f[2] = Some(bad.to_owned()));
        assert!(
            insert(artifact, result, SECOND, as_refs(&spaced)).is_err(),
            "{bad}"
        );
    }
    let negative = fields(&|f| f[3] = Some("-1".to_owned()));
    assert!(insert(artifact, result, SECOND, as_refs(&negative)).is_err());
    assert!(
        insert(
            artifact,
            "rollout_opening_session_meta",
            SECOND,
            as_refs(&full)
        )
        .is_err()
    );
    let foreign = fields(&|f| f[1] = Some(first(CHILD)));
    assert!(insert(artifact, result, SECOND, as_refs(&foreign)).is_err());
    let native_child = codex(OTHER_PARENT);
    for index in 0..8 {
        let mut one = [None; 8];
        one[index] = as_refs(&full)[index];
        assert!(
            insert(
                "codex_thread_spawn",
                "rollout_opening_session_meta",
                &native_child,
                one
            )
            .is_err(),
            "{index}"
        );
    }
    insert(artifact, result, SECOND, as_refs(&full)).unwrap();

    // Immutable but for the one-way withholding.
    for statement in [
        "UPDATE session_creation_relations SET launch_call_id='x'",
        "UPDATE session_creation_relations SET first_record_uuid=NULL",
        "UPDATE session_creation_relations SET state='conflicted',process_session_id='2'",
        "UPDATE session_creation_relations SET provider_result_uuid='x'",
        "DELETE FROM session_creation_relations",
    ] {
        assert!(connection.execute(statement, []).is_err(), "{statement}");
    }
    connection
        .execute(
            "UPDATE session_creation_relations SET state='conflicted' WHERE child_session_id=?1",
            [CHILD],
        )
        .unwrap();
    assert!(
        connection
            .execute("UPDATE session_creation_relations SET state='accepted'", [])
            .is_err()
    );

    // The launch owners hold structural identifiers only, keep their first
    // child and only ever become disputed.
    let owner_columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('cli_artifact_launch_owners') ORDER BY cid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        owner_columns,
        [
            "parent_session_id",
            "launch_call_id",
            "launch_operation_index",
            "first_child_session_id",
            "disputed",
        ]
    );
    assert_eq!(
        owners(&directory),
        [(proof(CHILD).launch_call_id, CHILD.to_owned(), false)]
    );
    for bad in [
        "INSERT INTO cli_artifact_launch_owners VALUES ('p','a b',0,'c',0)",
        "INSERT INTO cli_artifact_launch_owners VALUES ('p','c',-1,'c',0)",
        "INSERT INTO cli_artifact_launch_owners VALUES ('p','c',0,'missing-session',0)",
        "INSERT INTO cli_artifact_launch_owners VALUES ('p','c',0,'c',2)",
        "UPDATE cli_artifact_launch_owners SET first_child_session_id='judge'",
        "UPDATE cli_artifact_launch_owners SET launch_call_id='x'",
        "UPDATE cli_artifact_launch_owners SET disputed=1,launch_operation_index=1",
        "DELETE FROM cli_artifact_launch_owners",
    ] {
        assert!(connection.execute(bad, []).is_err(), "{bad}");
    }
    connection
        .execute("UPDATE cli_artifact_launch_owners SET disputed=1", [])
        .unwrap();
    assert!(
        connection
            .execute("UPDATE cli_artifact_launch_owners SET disputed=0", [])
            .is_err()
    );
}

/// A foreground CLI relation names its exact canonical parent; a read shows it
/// only when the parent's host and native identity resolve to exactly that
/// session.
#[test]
fn a_read_shows_only_the_named_canonical_parent() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    apply(&mut store, &[proof(CHILD)]);
    let connection = sql(&directory);
    // A row whose canonical parent disagrees with its native parent can only
    // be forged, and is never shown.
    connection
        .execute(
            "INSERT INTO session_creation_relations(child_session_id,child_host,
                 child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
                 evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
                 launch_call_id,launch_operation_index,process_session_id,completion_call_id,
                 output_read_call_id,provider_result_uuid)
             VALUES (?1,'claude',?1,'codex',?2,'cli_artifact_create',1,
                 'claude_cli_redirected_json_result','accepted',0,?3,?4,'c',0,'1','d','r','u')",
            rusqlite::params![SECOND, PARENT, codex(OTHER_PARENT), first(SECOND)],
        )
        .unwrap();
    let parents = session_list::parents(&connection, &[CHILD, SECOND]).unwrap();
    assert_eq!(parents.len(), 1);
    assert_eq!(parents[CHILD].session_id, codex(PARENT));
    assert_eq!(
        parents[CHILD].evidence,
        CreationEvidence::CliArtifactCreate.into()
    );
    let context = session_list::context(&connection, &[CHILD, SECOND]).unwrap();
    assert_eq!(
        context.len(),
        2,
        "the context holds exactly the named sessions"
    );
    assert_eq!(shown(&store), [(CHILD.to_owned(), codex(PARENT))]);
}

/// Every page's row identities, following the cursor to the end.
fn pages(store: &Store) -> Vec<Vec<String>> {
    let mut all = Vec::new();
    let mut after = None;
    loop {
        let mut rows = store
            .sessions_page_filtered(&SessionFilter::default(), after.as_ref())
            .unwrap();
        let more = rows.len() > 50;
        rows.truncate(50);
        after = rows.last().map(|row| row.cursor.clone());
        all.push(rows.into_iter().map(|row| row.id).collect());
        if !more {
            return all;
        }
    }
}
