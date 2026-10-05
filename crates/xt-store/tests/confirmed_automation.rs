//! Structural confirmations of automated inputs: one guarded, immutable,
//! content-free fact per saved input, applied by identity and read through the
//! shared record projection without touching the raw classification, the
//! record's identity or any of the session's actual work.

use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    confirmation::{
        Abstention, AutomatedInputProof, ConfirmationDisposition, EvidenceKind, MAX_CONFIRMATIONS,
    },
};

const SESSION: &str = "synthetic-target";
const NATIVE: &str = "synthetic-target";
const OTHER: &str = "synthetic-other";

fn record(value: serde_json::Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}

/// A dispatched input, the agent's work it caused and every kind of user-role
/// record that is not an input a person could have submitted.
fn records() -> Vec<CanonicalRecord> {
    vec![
        record(
            json!({"uuid":"input","type":"user","timestamp":"2026-09-07T12:00:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic dispatched request"}]}}),
        ),
        record(
            json!({"uuid":"second","type":"user","timestamp":"2026-09-07T12:10:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic second request"}]}}),
        ),
        record(
            json!({"uuid":"agent","type":"assistant","timestamp":"2026-09-07T12:01:00Z",
            "message":{"id":"msg","role":"assistant","model":"synthetic-model",
                "content":[{"type":"tool_use","name":"Read","input":{"path":"synthetic"}}],
                "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
        ),
        record(
            json!({"uuid":"result","type":"user","timestamp":"2026-09-07T12:02:00Z",
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"synthetic"}]}}),
        ),
        record(
            json!({"uuid":"meta","type":"user","isMeta":true,"timestamp":"2026-09-07T12:03:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic context"}]}}),
        ),
        record(
            json!({"uuid":"side","type":"user","isSidechain":true,"timestamp":"2026-09-07T12:04:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic sidechain"}]}}),
        ),
        record(
            json!({"uuid":"command","type":"user","timestamp":"2026-09-07T12:05:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"<command-name>/synthetic</command-name>"}]}}),
        ),
        record(
            json!({"uuid":"unknown","type":"user","timestamp":"2026-09-07T12:06:00Z",
            "message":{"role":"user"}}),
        ),
    ]
}

fn open(directory: &TempDir) -> Store {
    Store::open(directory.path().join("confirmations.sqlite")).unwrap()
}

fn seed(store: &mut Store) {
    let mut session = SessionMeta::new(SESSION, "claude", SessionSource::Transcript);
    session.native_session_id = Some(NATIVE.into());
    session.surface = Some("cli".into());
    store.upsert_session(&session, false).unwrap();
    store.upsert_records(SESSION, &records(), false).unwrap();
    let mut other = SessionMeta::new(OTHER, "claude", SessionSource::Transcript);
    other.native_session_id = Some(OTHER.into());
    store.upsert_session(&other, false).unwrap();
    store
        .upsert_records(
            OTHER,
            &[record(
                json!({"uuid":"elsewhere","type":"user","timestamp":"2026-09-07T12:00:00Z",
                "message":{"role":"user","content":[{"type":"text","text":"Synthetic other"}]}}),
            )],
            false,
        )
        .unwrap();
}

fn proof(uuid: &str, call: &str) -> AutomatedInputProof {
    AutomatedInputProof {
        record_uuid: uuid.into(),
        session_id: SESSION.into(),
        native_session_id: NATIVE.into(),
        parent_host: Host::Codex,
        parent_session_id: "codex-synthetic-parent".into(),
        parent_tool_call_id: call.into(),
        parent_operation_index: 0,
        parent_result_id: Some(format!("{call}-output")),
        evidence_kind: EvidenceKind::AgentDispatch,
        matcher_version: 1,
    }
}

/// `(uuid, effective is_human, raw is_human, confirmed)` as the shared
/// projection every metric reads states them.
fn projected(path: &std::path::Path) -> Vec<(String, Option<i64>, Option<i64>, i64)> {
    Connection::open(path)
        .unwrap()
        .prepare(
            "SELECT uuid,is_human,raw_is_human,confirmed_automated_input FROM v_records
             WHERE session_id=?1 ORDER BY uuid",
        )
        .unwrap()
        .query_map([SESSION], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// Everything about the session's records that is not the confirmation: the
/// raw rows, their usage and tool calls, compared before and after.
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

#[test]
fn confirmation_corrects_one_saved_human_input_and_survives_replay_and_reopen() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("confirmations.sqlite");
    let mut store = open(&directory);
    seed(&mut store);
    let before = work(&path);
    let untouched = projected(&path);
    assert!(untouched.contains(&("input".into(), Some(1), Some(1), 0)));

    let report = store
        .apply_automated_input_confirmations(&[proof("input", "call-launch")], 1_000)
        .unwrap();
    assert_eq!(report.dispositions, [ConfirmationDisposition::Confirmed]);
    assert_eq!(report.affected_sessions.len(), 1);
    assert_eq!(report.affected_sessions[0].session_id, SESSION);
    assert_eq!(report.affected_sessions[0].surface.as_deref(), Some("cli"));

    let corrected = projected(&path);
    for row in &corrected {
        if row.0 == "input" {
            // Not human any more, still the raw human observation, confirmed.
            assert_eq!(row, &("input".into(), Some(0), Some(1), 1));
        } else {
            assert!(untouched.contains(row), "{row:?}");
        }
    }
    // The record, its usage, its session and all agent work are unchanged.
    assert_eq!(work(&path), before);
    let typed = store
        .records(SESSION)
        .unwrap()
        .into_iter()
        .find(|r| r.uuid == "input")
        .unwrap();
    assert!(typed.confirmed_automated_input);
    assert_eq!(typed.classification.is_human, Some(true));

    // Repeating the identical proof is a no-op.
    let again = store
        .apply_automated_input_confirmations(&[proof("input", "call-launch")], 2_000)
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
    assert_eq!(work(&path), before);
    drop(store);

    let reopened = open(&directory);
    assert_eq!(projected(&path), corrected);
    assert_eq!(work(&path), before);
    assert_eq!(
        reopened.automated_input_confirmations(SESSION).unwrap(),
        [proof("input", "call-launch")]
    );
    // Stored once, at its first application time.
    let applied: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT confirmed_at FROM confirmed_automated_inputs",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(applied, 1_000);
}

#[test]
fn confirmation_table_holds_only_structural_identities_and_is_immutable() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("confirmations.sqlite");
    let mut store = open(&directory);
    seed(&mut store);
    store
        .apply_automated_input_confirmations(&[proof("input", "call-launch")], 1_000)
        .unwrap();
    let sql = Connection::open(&path).unwrap();
    let columns: Vec<String> = sql
        .prepare("SELECT name FROM pragma_table_info('confirmed_automated_inputs') ORDER BY cid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        columns,
        [
            "record_uuid",
            "session_id",
            "native_session_id",
            "evidence_kind",
            "matcher_version",
            "parent_host",
            "parent_session_id",
            "parent_tool_call_id",
            "parent_operation_index",
            "parent_result_id",
            "confirmed_at"
        ]
    );
    let stored: String = sql
        .query_row(
            "SELECT json_array(record_uuid,session_id,native_session_id,evidence_kind,matcher_version,
                 parent_host,parent_session_id,parent_tool_call_id,parent_operation_index,parent_result_id)
             FROM confirmed_automated_inputs",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!stored.contains("Synthetic"), "{stored}");
    // Metadata-only storage retained no content anywhere.
    let content: i64 = sql
        .query_row(
            "SELECT count(*) FROM records WHERE content_json IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(content, 0);
    for statement in [
        "UPDATE confirmed_automated_inputs SET record_uuid='second'",
        "UPDATE confirmed_automated_inputs SET parent_tool_call_id='other'",
        "DELETE FROM confirmed_automated_inputs",
    ] {
        assert!(sql.execute(statement, []).is_err(), "{statement}");
    }
    // A row naming a record in another session cannot be written at all.
    assert!(
        sql.execute(
            "INSERT INTO confirmed_automated_inputs VALUES('elsewhere',?1,?1,'agent_dispatch',1,'codex','p','c',0,NULL,1)",
            [SESSION],
        )
        .is_err()
    );
    assert_eq!(
        store.automated_input_confirmations(SESSION).unwrap(),
        [proof("input", "call-launch")]
    );
}

#[test]
fn ineligible_or_unguarded_targets_abstain_without_writing() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("confirmations.sqlite");
    let mut store = open(&directory);
    seed(&mut store);
    let before = projected(&path);
    let wrong_session = AutomatedInputProof {
        session_id: OTHER.into(),
        native_session_id: OTHER.into(),
        ..proof("input", "call-wrong-session")
    };
    let wrong_native = AutomatedInputProof {
        native_session_id: "synthetic-different-native".into(),
        ..proof("input", "call-wrong-native")
    };
    let cases = [
        (proof("absent", "call-absent"), Abstention::MissingTarget),
        (wrong_session, Abstention::SessionMismatch),
        (wrong_native, Abstention::NativeIdentityMismatch),
        (proof("agent", "call-agent"), Abstention::NotUserInput),
        (proof("result", "call-result"), Abstention::NotUserInput),
        (proof("meta", "call-meta"), Abstention::NotUserInput),
        (proof("side", "call-side"), Abstention::NotUserInput),
        (
            proof("command", "call-command"),
            Abstention::NotHumanClassified,
        ),
        (
            proof("unknown", "call-unknown"),
            Abstention::NotHumanClassified,
        ),
    ];
    // One call per case: two proofs naming one input in a single call would
    // be refused as ambiguous before any target is inspected.
    for (proof, reason) in cases {
        let report = store
            .apply_automated_input_confirmations(&[proof], 1_000)
            .unwrap();
        assert_eq!(
            report.dispositions,
            [ConfirmationDisposition::Abstained(reason)]
        );
        assert!(report.affected_sessions.is_empty());
    }
    assert_eq!(projected(&path), before);
    assert!(
        store
            .automated_input_confirmations(SESSION)
            .unwrap()
            .is_empty()
    );

    // A session whose native identity is unknown cannot be guarded.
    let sql = Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE sessions SET native_session_id=NULL WHERE session_id=?1",
        [SESSION],
    )
    .unwrap();
    assert_eq!(
        store
            .apply_automated_input_confirmations(&[proof("input", "call-launch")], 1_000)
            .unwrap()
            .dispositions,
        [ConfirmationDisposition::Abstained(
            Abstention::NativeIdentityMismatch
        )]
    );
    sql.execute(
        "UPDATE sessions SET native_session_id=?1,kind='judge' WHERE session_id=?1",
        [SESSION],
    )
    .unwrap();
    assert_eq!(
        store
            .apply_automated_input_confirmations(&[proof("input", "call-launch")], 1_000)
            .unwrap()
            .dispositions,
        [ConfirmationDisposition::Abstained(Abstention::NotUserInput)]
    );
    assert!(
        store
            .automated_input_confirmations(SESSION)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn conflicting_or_ambiguous_proofs_never_overwrite_or_transfer() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("confirmations.sqlite");
    let mut store = open(&directory);
    seed(&mut store);

    // Inside one batch: two different proofs for one input, and one parent
    // operation naming two inputs, confirm nothing. An exact repeat is fine.
    let other_call = proof("input", "call-other");
    let report = store
        .apply_automated_input_confirmations(
            &[
                proof("input", "call-launch"),
                other_call.clone(),
                proof("second", "call-shared"),
                AutomatedInputProof {
                    record_uuid: "input".into(),
                    ..proof("second", "call-shared")
                },
            ],
            1_000,
        )
        .unwrap();
    assert_eq!(
        report.dispositions,
        [ConfirmationDisposition::Abstained(Abstention::AmbiguousInBatch); 4]
    );
    assert!(
        store
            .automated_input_confirmations(SESSION)
            .unwrap()
            .is_empty()
    );
    let report = store
        .apply_automated_input_confirmations(
            &[proof("input", "call-launch"), proof("input", "call-launch")],
            1_000,
        )
        .unwrap();
    assert_eq!(
        report.dispositions,
        [
            ConfirmationDisposition::Confirmed,
            ConfirmationDisposition::AlreadyConfirmed
        ]
    );

    // Across calls: a different proof for the confirmed input is refused, and
    // the stored proof's parent operation cannot confirm another input.
    let changed_result = AutomatedInputProof {
        parent_result_id: None,
        ..proof("input", "call-launch")
    };
    let reused = AutomatedInputProof {
        record_uuid: "second".into(),
        ..proof("input", "call-launch")
    };
    for conflicting in [other_call, changed_result, reused] {
        assert_eq!(
            store
                .apply_automated_input_confirmations(&[conflicting], 2_000)
                .unwrap()
                .dispositions,
            [ConfirmationDisposition::Abstained(
                Abstention::ConflictingConfirmation
            )]
        );
    }
    assert_eq!(
        store.automated_input_confirmations(SESSION).unwrap(),
        [proof("input", "call-launch")]
    );
    // The same call's next operation is a distinct, valid proof.
    let resumed = AutomatedInputProof {
        record_uuid: "second".into(),
        parent_operation_index: 2,
        ..proof("input", "call-launch")
    };
    assert_eq!(
        store
            .apply_automated_input_confirmations(std::slice::from_ref(&resumed), 3_000)
            .unwrap()
            .dispositions,
        [ConfirmationDisposition::Confirmed]
    );
    assert_eq!(
        projected(&path)
            .into_iter()
            .filter(|row| row.3 == 1)
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        ["input", "second"]
    );
}

#[test]
fn malformed_or_unbounded_input_rejects_the_whole_call() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    seed(&mut store);
    let valid = proof("input", "call-launch");
    for broken in [
        AutomatedInputProof {
            parent_tool_call_id: " ".into(),
            ..valid.clone()
        },
        AutomatedInputProof {
            parent_session_id: "line\nbreak".into(),
            ..valid.clone()
        },
        AutomatedInputProof {
            record_uuid: "x".repeat(257),
            ..valid.clone()
        },
        AutomatedInputProof {
            parent_result_id: Some(String::new()),
            ..valid.clone()
        },
        AutomatedInputProof {
            matcher_version: 0,
            ..valid.clone()
        },
    ] {
        assert!(
            store
                .apply_automated_input_confirmations(&[valid.clone(), broken], 1_000)
                .is_err()
        );
    }
    let many = vec![valid.clone(); MAX_CONFIRMATIONS + 1];
    assert!(
        store
            .apply_automated_input_confirmations(&many, 1_000)
            .is_err()
    );
    assert!(
        store
            .automated_input_confirmations(SESSION)
            .unwrap()
            .is_empty()
    );
}
