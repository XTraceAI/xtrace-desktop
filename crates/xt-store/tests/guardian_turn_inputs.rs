//! Guardian turn confirmations: one guarded, immutable, content-free fact that
//! a saved Codex input is the input a parent thread's exact turn dispatched to
//! a Guardian reviewer turn. It is kept apart from tool-call confirmations, is
//! read through the same record projection, and never touches the raw
//! classification, the record's identity, the rest of its session or any work.

use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    confirmation::{
        Abstention, AutomatedInputProof, ConfirmationDisposition, EvidenceKind,
        GuardianEvidenceKind, GuardianTurnProof, MAX_CONFIRMATIONS,
    },
};

// Synthetic canonical identities; none names a real thread or turn.
const NATIVE: &str = "00000000-0000-7000-8000-00000000aaaa";
const SESSION: &str = "codex-00000000-0000-7000-8000-00000000aaaa";
const OTHER_NATIVE: &str = "00000000-0000-7000-8000-00000000bbbb";
const OTHER: &str = "codex-00000000-0000-7000-8000-00000000bbbb";
const PLUGIN_NATIVE: &str = "00000000-0000-7000-8000-00000000cccc";
const PLUGIN: &str = "codex-00000000-0000-7000-8000-00000000cccc";
const PARENT: &str = "00000000-0000-7000-8000-00000000dddd";

/// A synthetic canonical UUID ending in `n`.
fn id(n: u32) -> String {
    format!("abcdef00-0000-5000-8000-{n:012x}")
}
fn turn(n: u32) -> String {
    format!("abcdef00-0000-7000-9000-{n:012x}")
}
fn parent_turn(n: u32) -> String {
    format!("abcdef00-0000-7000-a000-{n:012x}")
}

const INPUT: u32 = 1;
const SECOND: u32 = 2;
const AGENT: u32 = 3;
const RESULT: u32 = 4;
const META: u32 = 5;
const SIDE: u32 = 6;
const COMMAND: u32 = 7;
const UNKNOWN: u32 = 8;
const CONFLICTED: u32 = 9;
const ELSEWHERE: u32 = 10;
const PLUGIN_INPUT: u32 = 11;

fn record(value: serde_json::Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn input(n: u32, minute: u32) -> CanonicalRecord {
    record(
        json!({"uuid":id(n),"type":"user","timestamp":format!("2026-09-07T12:{minute:02}:00Z"),
        "message":{"role":"user","content":[{"type":"text","text":"Synthetic reviewer request"}]}}),
    )
}

/// Two reviewer inputs, the agent work around them and every kind of
/// user-role record that is not an input a person could have submitted.
fn records() -> Vec<CanonicalRecord> {
    vec![
        input(INPUT, 0),
        input(SECOND, 10),
        record(
            json!({"uuid":id(AGENT),"type":"assistant","timestamp":"2026-09-07T12:01:00Z",
            "message":{"id":"msg","role":"assistant","model":"synthetic-model",
                "content":[{"type":"tool_use","name":"Read","input":{"path":"synthetic"}}],
                "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
        ),
        record(
            json!({"uuid":id(RESULT),"type":"user","timestamp":"2026-09-07T12:02:00Z",
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"synthetic"}]}}),
        ),
        record(
            json!({"uuid":id(META),"type":"user","isMeta":true,"timestamp":"2026-09-07T12:03:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic context"}]}}),
        ),
        record(
            json!({"uuid":id(SIDE),"type":"user","isSidechain":true,"timestamp":"2026-09-07T12:04:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic sidechain"}]}}),
        ),
        record(
            json!({"uuid":id(COMMAND),"type":"user","timestamp":"2026-09-07T12:05:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"<command-name>/synthetic</command-name>"}]}}),
        ),
        record(
            json!({"uuid":id(UNKNOWN),"type":"user","timestamp":"2026-09-07T12:06:00Z",
            "message":{"role":"user"}}),
        ),
        input(CONFLICTED, 7),
    ]
}

fn open(directory: &TempDir) -> Store {
    Store::open(directory.path().join("guardian.sqlite")).unwrap()
}

fn session(store: &mut Store, id: &str, native: &str, source: SessionSource) {
    let mut meta = SessionMeta::new(id, "codex", source);
    meta.native_session_id = Some(native.into());
    meta.surface = Some("cli".into());
    store.upsert_session(&meta, false).unwrap();
}

fn seed(store: &mut Store, path: &std::path::Path) {
    session(store, SESSION, NATIVE, SessionSource::ReadersCli);
    store.upsert_records(SESSION, &records(), false).unwrap();
    session(store, OTHER, OTHER_NATIVE, SessionSource::ReadersCli);
    store
        .upsert_records(OTHER, &[input(ELSEWHERE, 0)], false)
        .unwrap();
    // Codex history that did not come from the native reader.
    session(store, PLUGIN, PLUGIN_NATIVE, SessionSource::Plugin);
    store
        .upsert_records(PLUGIN, &[input(PLUGIN_INPUT, 0)], false)
        .unwrap();
    // A record whose saved copies disagreed, as ingestion marks one.
    Connection::open(path)
        .unwrap()
        .execute(
            "UPDATE records SET has_conflict=1 WHERE uuid=?1",
            [id(CONFLICTED)],
        )
        .unwrap();
}

fn proof(n: u32, t: u32) -> GuardianTurnProof {
    GuardianTurnProof {
        record_uuid: id(n),
        session_id: SESSION.into(),
        native_session_id: NATIVE.into(),
        turn_id: turn(t),
        parent_native_session_id: PARENT.into(),
        parent_turn_id: parent_turn(t),
        evidence_kind: GuardianEvidenceKind::GuardianTurnDispatch,
        matcher_version: 1,
    }
}

/// `(uuid, effective is_human, raw is_human, confirmed)` for every session.
fn projected(path: &std::path::Path) -> Vec<(String, Option<i64>, Option<i64>, i64)> {
    Connection::open(path)
        .unwrap()
        .prepare(
            "SELECT uuid,is_human,raw_is_human,confirmed_automated_input FROM v_records
             ORDER BY uuid",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// Everything that is not a confirmation: raw rows, usage, tools, sessions.
fn work(path: &std::path::Path) -> Vec<String> {
    let sql = Connection::open(path).unwrap();
    let mut rows = Vec::new();
    for query in [
        "SELECT json_array(uuid,session_id,type,ts,ts_ms,api_message_id,is_meta,is_sidechain,role,model,
             is_tool_result_carrier,text_len,tool_use_count,content_json,has_conflict,is_human,is_command)
         FROM records ORDER BY uuid",
        "SELECT json_array(uuid,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens) FROM usage ORDER BY uuid",
        "SELECT json_array(uuid,block_index,name,input_json,kind) FROM tool_uses ORDER BY uuid,block_index",
        "SELECT json_array(session_id,record_count,first_ts,last_ts,has_conflict) FROM sessions ORDER BY session_id",
    ] {
        rows.extend(
            sql.prepare(query)
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap),
        );
    }
    rows
}

fn count(path: &std::path::Path, table: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn dispositions(store: &mut Store, proofs: &[GuardianTurnProof]) -> Vec<ConfirmationDisposition> {
    store
        .apply_guardian_turn_confirmations(proofs, 1_000)
        .unwrap()
        .dispositions
}

#[test]
fn one_turn_proof_corrects_exactly_its_input_and_survives_replay_and_reopen() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("guardian.sqlite");
    let mut store = open(&directory);
    seed(&mut store, &path);
    let before = work(&path);
    let untouched = projected(&path);
    assert!(untouched.contains(&(id(INPUT), Some(1), Some(1), 0)));
    assert!(untouched.contains(&(id(SECOND), Some(1), Some(1), 0)));

    let report = store
        .apply_guardian_turn_confirmations(&[proof(INPUT, 1)], 1_000)
        .unwrap();
    assert_eq!(report.dispositions, [ConfirmationDisposition::Confirmed]);
    assert_eq!(report.affected_sessions.len(), 1);
    assert_eq!(report.affected_sessions[0].session_id, SESSION);
    assert_eq!(report.affected_sessions[0].surface.as_deref(), Some("cli"));

    // Only the proven input changes; the reviewer session's other input, and
    // every other record, keep exactly their previous projection.
    let corrected = projected(&path);
    for row in &corrected {
        if row.0 == id(INPUT) {
            assert_eq!(row, &(id(INPUT), Some(0), Some(1), 1));
        } else {
            assert!(untouched.contains(row), "{row:?}");
        }
    }
    assert_eq!(work(&path), before);
    // The tool-call confirmation table is not used for a turn link.
    assert_eq!(count(&path, "confirmed_automated_inputs"), 0);
    let typed = store.records(SESSION).unwrap();
    for record in &typed {
        assert_eq!(record.confirmed_automated_input, record.uuid == id(INPUT));
    }
    assert_eq!(
        typed
            .iter()
            .find(|r| r.uuid == id(INPUT))
            .unwrap()
            .classification
            .is_human,
        Some(true)
    );

    // Repeating the identical proof is a no-op that invalidates nothing.
    let again = store
        .apply_guardian_turn_confirmations(&[proof(INPUT, 1)], 2_000)
        .unwrap();
    assert_eq!(
        again.dispositions,
        [ConfirmationDisposition::AlreadyConfirmed]
    );
    assert!(again.affected_sessions.is_empty());

    // Ordinary replay lacking any evidence neither restores nor duplicates.
    let stats = store.upsert_records(SESSION, &records(), false).unwrap();
    assert_eq!((stats.inserted, stats.enriched), (0, 0));
    assert_eq!(projected(&path), corrected);
    drop(store);

    let mut reopened = open(&directory);
    assert_eq!(projected(&path), corrected);
    assert_eq!(work(&path), before);
    assert_eq!(
        reopened.guardian_turn_confirmations(SESSION).unwrap(),
        [proof(INPUT, 1)]
    );
    assert!(
        reopened
            .automated_input_confirmations(SESSION)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        dispositions(&mut reopened, &[proof(INPUT, 1)]),
        [ConfirmationDisposition::AlreadyConfirmed]
    );
    let applied: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT confirmed_at FROM guardian_turn_inputs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(applied, 1_000);

    // The session's second input needs its own turn proof.
    let second = reopened
        .apply_guardian_turn_confirmations(&[proof(SECOND, 2)], 3_000)
        .unwrap();
    assert_eq!(second.dispositions, [ConfirmationDisposition::Confirmed]);
    assert_eq!(second.affected_sessions.len(), 1);
    assert_eq!(count(&path, "guardian_turn_inputs"), 2);
}

#[test]
fn the_table_holds_only_canonical_structural_identities_and_is_immutable() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("guardian.sqlite");
    let mut store = open(&directory);
    seed(&mut store, &path);
    dispositions(&mut store, &[proof(INPUT, 1)]);
    let sql = Connection::open(&path).unwrap();
    let columns: Vec<(String, String)> = sql
        .prepare("SELECT name,type FROM pragma_table_info('guardian_turn_inputs') ORDER BY cid")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        columns,
        [
            ("record_uuid", "TEXT"),
            ("session_id", "TEXT"),
            ("native_session_id", "TEXT"),
            ("turn_id", "TEXT"),
            ("parent_native_session_id", "TEXT"),
            ("parent_turn_id", "TEXT"),
            ("evidence_kind", "TEXT"),
            ("matcher_version", "INTEGER"),
            ("confirmed_at", "INTEGER"),
        ]
        .map(|(name, kind)| (name.to_owned(), kind.to_owned()))
    );
    let stored: String = sql
        .query_row(
            "SELECT json_array(record_uuid,session_id,native_session_id,turn_id,
                 parent_native_session_id,parent_turn_id,evidence_kind,matcher_version)
             FROM guardian_turn_inputs",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!stored.contains("Synthetic"), "{stored}");
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM records WHERE content_json IS NOT NULL",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    for statement in [
        "UPDATE guardian_turn_inputs SET matcher_version=2",
        "UPDATE guardian_turn_inputs SET parent_turn_id=turn_id",
        "DELETE FROM guardian_turn_inputs",
    ] {
        assert!(sql.execute(statement, []).is_err(), "{statement}");
    }

    // Direct writes must still have the closed shape. Start from a valid row
    // for the second input and break one fact at a time.
    let valid = [
        id(SECOND),
        SESSION.to_owned(),
        NATIVE.to_owned(),
        turn(2),
        PARENT.to_owned(),
        parent_turn(2),
        "guardian_turn_dispatch".to_owned(),
    ];
    let insert = |values: &[String; 7], version: i64| {
        sql.execute(
            "INSERT INTO guardian_turn_inputs VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1)",
            rusqlite::params![
                values[0], values[1], values[2], values[3], values[4], values[5], values[6],
                version
            ],
        )
    };
    let mut broken = Vec::new();
    for (index, value) in [
        (0, id(SECOND).to_uppercase()),
        (0, format!("{{{}}}", id(SECOND))),
        (1, NATIVE.to_owned()),
        (1, OTHER.to_owned()),
        (3, "1".to_owned()),
        (4, NATIVE.to_owned()),
        (4, PARENT.to_uppercase()),
        (5, turn(2)),
        (5, " ".to_owned()),
        (6, "agent_dispatch".to_owned()),
        // A record in another session cannot be named.
        (0, id(ELSEWHERE)),
        // One Guardian turn confirms one input.
        (3, turn(1)),
    ] {
        let mut values = valid.clone();
        values[index] = value;
        broken.push(values);
    }
    for values in &broken {
        assert!(insert(values, 1).is_err(), "{values:?}");
    }
    assert!(insert(&valid, 0).is_err());
    assert_eq!(
        store.guardian_turn_confirmations(SESSION).unwrap(),
        [proof(INPUT, 1)]
    );
    // Each refusal above was its one broken fact: the unbroken row is legal.
    assert_eq!(insert(&valid, 1).unwrap(), 1);
    assert_eq!(
        store.guardian_turn_confirmations(SESSION).unwrap(),
        [proof(INPUT, 1), proof(SECOND, 2)]
    );
}

#[test]
fn ineligible_or_unguarded_targets_abstain_without_writing() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("guardian.sqlite");
    let mut store = open(&directory);
    seed(&mut store, &path);
    let before = projected(&path);
    let wrong_session = GuardianTurnProof {
        session_id: OTHER.into(),
        native_session_id: OTHER_NATIVE.into(),
        ..proof(INPUT, 1)
    };
    let not_reader = GuardianTurnProof {
        session_id: PLUGIN.into(),
        native_session_id: PLUGIN_NATIVE.into(),
        ..proof(PLUGIN_INPUT, 1)
    };
    let cases = [
        (proof(99, 1), Abstention::MissingTarget),
        (wrong_session, Abstention::SessionMismatch),
        (not_reader, Abstention::NotCodexReaderHistory),
        (proof(AGENT, 1), Abstention::NotUserInput),
        (proof(RESULT, 1), Abstention::NotUserInput),
        (proof(META, 1), Abstention::NotUserInput),
        (proof(SIDE, 1), Abstention::NotUserInput),
        (proof(COMMAND, 1), Abstention::NotHumanClassified),
        (proof(UNKNOWN, 1), Abstention::NotHumanClassified),
        (proof(CONFLICTED, 1), Abstention::ConflictedRecord),
    ];
    for (proof, reason) in cases {
        let report = store
            .apply_guardian_turn_confirmations(&[proof], 1_000)
            .unwrap();
        assert_eq!(
            report.dispositions,
            [ConfirmationDisposition::Abstained(reason)]
        );
        assert!(report.affected_sessions.is_empty());
    }
    assert_eq!(projected(&path), before);
    assert_eq!(count(&path, "guardian_turn_inputs"), 0);

    let sql = Connection::open(&path).unwrap();
    // Any other host, and an unknown or different native identity.
    sql.execute(
        "UPDATE sessions SET host='claude' WHERE session_id=?1",
        [SESSION],
    )
    .unwrap();
    assert_eq!(
        dispositions(&mut store, &[proof(INPUT, 1)]),
        [ConfirmationDisposition::Abstained(
            Abstention::NotCodexReaderHistory
        )]
    );
    sql.execute(
        "UPDATE sessions SET host='codex',native_session_id=NULL WHERE session_id=?1",
        [SESSION],
    )
    .unwrap();
    assert_eq!(
        dispositions(&mut store, &[proof(INPUT, 1)]),
        [ConfirmationDisposition::Abstained(
            Abstention::NativeIdentityMismatch
        )]
    );
    sql.execute(
        "UPDATE sessions SET native_session_id=?2 WHERE session_id=?1",
        [SESSION, OTHER_NATIVE],
    )
    .unwrap();
    assert_eq!(
        dispositions(&mut store, &[proof(INPUT, 1)]),
        [ConfirmationDisposition::Abstained(
            Abstention::NativeIdentityMismatch
        )]
    );
    sql.execute(
        "UPDATE sessions SET native_session_id=?2,kind='judge' WHERE session_id=?1",
        [SESSION, NATIVE],
    )
    .unwrap();
    assert_eq!(
        dispositions(&mut store, &[proof(INPUT, 1)]),
        [ConfirmationDisposition::Abstained(Abstention::NotUserInput)]
    );
    assert_eq!(count(&path, "guardian_turn_inputs"), 0);
}

#[test]
fn conflicting_or_ambiguous_proofs_never_overwrite_transfer_or_cross_tables() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("guardian.sqlite");
    let mut store = open(&directory);
    seed(&mut store, &path);

    // Inside one batch: one input under two turns, and one turn naming two
    // inputs, confirm nothing. An exact repeat is fine.
    let other_parent = GuardianTurnProof {
        parent_turn_id: parent_turn(9),
        ..proof(INPUT, 1)
    };
    assert_eq!(
        dispositions(
            &mut store,
            &[
                proof(INPUT, 1),
                other_parent.clone(),
                proof(SECOND, 2),
                GuardianTurnProof {
                    record_uuid: id(CONFLICTED),
                    ..proof(SECOND, 2)
                },
            ]
        ),
        [ConfirmationDisposition::Abstained(Abstention::AmbiguousInBatch); 4]
    );
    assert_eq!(count(&path, "guardian_turn_inputs"), 0);
    assert_eq!(
        dispositions(&mut store, &[proof(INPUT, 1), proof(INPUT, 1)]),
        [
            ConfirmationDisposition::Confirmed,
            ConfirmationDisposition::AlreadyConfirmed
        ]
    );

    // Across calls: a different proof for the confirmed input, and the
    // stored turn naming another input, are refused.
    let other_turn = GuardianTurnProof {
        turn_id: turn(5),
        ..proof(INPUT, 1)
    };
    let other_version = GuardianTurnProof {
        matcher_version: 2,
        ..proof(INPUT, 1)
    };
    let reused_turn = GuardianTurnProof {
        record_uuid: id(SECOND),
        ..proof(INPUT, 1)
    };
    for conflicting in [other_parent, other_turn, other_version, reused_turn] {
        assert_eq!(
            dispositions(&mut store, &[conflicting]),
            [ConfirmationDisposition::Abstained(
                Abstention::ConflictingConfirmation
            )]
        );
    }
    assert_eq!(
        store.guardian_turn_confirmations(SESSION).unwrap(),
        [proof(INPUT, 1)]
    );

    // A tool-call confirmation and a turn confirmation never both hold one
    // input, whichever arrives first.
    let dispatch = |n: u32, call: &str| AutomatedInputProof {
        record_uuid: id(n),
        session_id: SESSION.into(),
        native_session_id: NATIVE.into(),
        parent_host: Host::Codex,
        parent_session_id: format!("codex-{PARENT}"),
        parent_tool_call_id: call.into(),
        parent_operation_index: 0,
        parent_result_id: None,
        evidence_kind: EvidenceKind::AgentDispatch,
        matcher_version: 1,
    };
    assert_eq!(
        store
            .apply_automated_input_confirmations(&[dispatch(INPUT, "call-one")], 2_000)
            .unwrap()
            .dispositions,
        [ConfirmationDisposition::Abstained(
            Abstention::ConflictingConfirmation
        )]
    );
    assert_eq!(
        store
            .apply_automated_input_confirmations(&[dispatch(SECOND, "call-two")], 2_000)
            .unwrap()
            .dispositions,
        [ConfirmationDisposition::Confirmed]
    );
    let dispatched = store.automated_input_confirmations(SESSION).unwrap();
    assert_eq!(dispatched, [dispatch(SECOND, "call-two")]);
    assert_eq!(
        dispositions(&mut store, &[proof(SECOND, 2)]),
        [ConfirmationDisposition::Abstained(
            Abstention::ConflictingConfirmation
        )]
    );
    assert_eq!(
        store.guardian_turn_confirmations(SESSION).unwrap(),
        [proof(INPUT, 1)]
    );
    assert_eq!(
        projected(&path)
            .into_iter()
            .filter(|row| row.3 == 1)
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        [id(INPUT), id(SECOND)]
    );
}

#[test]
fn malformed_self_linked_or_unbounded_input_rejects_the_whole_call() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("guardian.sqlite");
    let mut store = open(&directory);
    seed(&mut store, &path);
    let valid = proof(INPUT, 1);
    for broken in [
        GuardianTurnProof {
            record_uuid: id(INPUT).to_uppercase(),
            ..valid.clone()
        },
        GuardianTurnProof {
            record_uuid: "input".into(),
            ..valid.clone()
        },
        GuardianTurnProof {
            turn_id: "1".into(),
            ..valid.clone()
        },
        GuardianTurnProof {
            turn_id: format!("{} ", turn(1)),
            ..valid.clone()
        },
        GuardianTurnProof {
            parent_turn_id: parent_turn(1).replace('-', ""),
            ..valid.clone()
        },
        GuardianTurnProof {
            session_id: NATIVE.into(),
            ..valid.clone()
        },
        GuardianTurnProof {
            session_id: OTHER.into(),
            ..valid.clone()
        },
        GuardianTurnProof {
            native_session_id: format!("codex-{NATIVE}"),
            ..valid.clone()
        },
        // The parent is the reviewer itself, or its own turn.
        GuardianTurnProof {
            parent_native_session_id: NATIVE.into(),
            ..valid.clone()
        },
        GuardianTurnProof {
            parent_turn_id: turn(1),
            ..valid.clone()
        },
        GuardianTurnProof {
            matcher_version: 0,
            ..valid.clone()
        },
    ] {
        assert!(
            store
                .apply_guardian_turn_confirmations(&[valid.clone(), broken.clone()], 1_000)
                .is_err(),
            "{broken:?}"
        );
    }
    let many = vec![valid.clone(); MAX_CONFIRMATIONS + 1];
    assert!(
        store
            .apply_guardian_turn_confirmations(&many, 1_000)
            .is_err()
    );
    assert_eq!(count(&path, "guardian_turn_inputs"), 0);
    assert_eq!(
        dispositions(&mut store, &vec![valid.clone(); MAX_CONFIRMATIONS]),
        [
            vec![ConfirmationDisposition::Confirmed],
            vec![ConfirmationDisposition::AlreadyConfirmed; MAX_CONFIRMATIONS - 1]
        ]
        .concat()
    );
}
