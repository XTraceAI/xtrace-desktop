//! Injected context proofs: one immutable, content-free source fact that a
//! saved Codex input is context Codex itself injected (a selected skill's
//! instructions), accepted only with that record's first insertion from the
//! same exact read. It is read through the shared record projection and never
//! touches the raw classification, the record, the rest of its session or any
//! work. A read of a record the index already held never acquires one.

use rusqlite::{Connection, types::Value as SqlValue};
use serde_json::json;
use tempfile::TempDir;
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    batch::{IngestBatch, IngestBatchOutcome, RecordDisposition},
    confirmation::{
        Abstention, AutomatedInputProof, ConfirmationDisposition, EvidenceKind,
        GuardianEvidenceKind, GuardianTurnProof,
    },
    ingest::DiscoveredSession,
    injected::{
        InjectedContextAbstention, InjectedContextDisposition, InjectedContextKind,
        InjectedContextOutcome, InjectedContextProof, OriginContract, SegmentHistory,
    },
};

// Synthetic canonical identities; none names a real session, item or turn.
const NATIVE: &str = "00000000-0000-7000-8000-00000000aaaa";
const SESSION: &str = "codex-00000000-0000-7000-8000-00000000aaaa";
const OTHER_NATIVE: &str = "00000000-0000-7000-8000-00000000bbbb";
const OTHER: &str = "codex-00000000-0000-7000-8000-00000000bbbb";
const ROLLOUT: &str = "00000000-0000-7000-8000-00000000cccc";
const PARENT: &str = "00000000-0000-7000-8000-00000000dddd";

/// A synthetic canonical record UUID ending in `n`.
fn id(n: u32) -> String {
    format!("abcdef00-0000-5000-8000-{n:012x}")
}
fn turn(n: u32) -> String {
    format!("abcdef00-0000-7000-9000-{n:012x}")
}
fn item(n: u32) -> String {
    format!("msg_abcdef00-0000-7000-a000-{n:012x}")
}

/// The person's own `$skill` request.
const ASK: u32 = 1;
/// The instructions Codex injected beside it.
const SKILL: u32 = 2;
const WORK: u32 = 3;
const RESULT: u32 = 4;
/// Text a person pasted that reads exactly like the injected body.
const PASTED: u32 = 5;
const NEW: u32 = 6;

const BODY: &str = "Synthetic selected skill instructions";

fn record(value: serde_json::Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn text(n: u32, second: u32, text: &str) -> CanonicalRecord {
    record(
        json!({"uuid":id(n),"type":"user","timestamp":format!("2026-09-07T12:00:{second:02}Z"),
        "message":{"role":"user","content":[{"type":"text","text":text}]}}),
    )
}

/// One exact read of the session: the person's request, the injected body,
/// the work it started, and a later pasted lookalike of the body.
fn records() -> Vec<CanonicalRecord> {
    vec![
        text(ASK, 0, "Use $synthetic-skill on this"),
        text(SKILL, 1, BODY),
        record(
            json!({"uuid":id(WORK),"type":"assistant","timestamp":"2026-09-07T12:00:02Z",
            "message":{"id":"msg-work","role":"assistant","model":"synthetic-model",
                "content":[{"type":"tool_use","name":"Read","input":{"path":"synthetic"}}],
                "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
        ),
        record(
            json!({"uuid":id(RESULT),"type":"user","timestamp":"2026-09-07T12:00:03Z",
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"synthetic"}]}}),
        ),
        text(PASTED, 4, BODY),
    ]
}

/// The reader's claim for the record ending in `n`, from a flat rollout.
fn proof(n: u32) -> InjectedContextProof {
    InjectedContextProof {
        contract: OriginContract::CodexOriginEvidence,
        version: 1,
        kind: InjectedContextKind::CodexSelectedSkillInstructions,
        native_session_id: NATIVE.into(),
        history: SegmentHistory::Flat,
        rollout_id: None,
        row_index: n + 3,
        row_ordinal: None,
        item_id: item(n),
        turn_id: turn(1),
        record_uuid: id(n),
    }
}

/// Each proof beside the input with its record UUID; other inputs carry none.
fn aligned(
    records: &[CanonicalRecord],
    proofs: &[InjectedContextProof],
) -> Vec<Option<InjectedContextProof>> {
    records
        .iter()
        .map(|record| {
            proofs
                .iter()
                .find(|proof| record.uuid.as_deref() == Some(proof.record_uuid.as_str()))
                .cloned()
        })
        .collect()
}

fn meta(session: &str, native: &str) -> SessionMeta {
    let mut meta = SessionMeta::new(session, "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some(native.into());
    meta.surface = Some("cli".into());
    meta
}

/// A discovered native Codex reader batch, as the importer submits one.
fn read(
    store: &mut Store,
    session: &str,
    native: &str,
    records: &[CanonicalRecord],
    proofs: &[Option<InjectedContextProof>],
) -> xt_store::Result<IngestBatchOutcome> {
    let meta = meta(session, native);
    let discovery = DiscoveredSession {
        host: Host::Codex,
        native_session_id: native.into(),
        conversation_id: Some(session.into()),
        surface: Some("cli".into()),
        started_at_ms: None,
        last_observed_at: 5_000,
        discovery_complete: true,
    };
    let mut batch = IngestBatch::new(&meta, records, false);
    batch.native_codex = true;
    batch.discovery = Some(&discovery);
    batch.injected_context = proofs;
    store.apply_ingest_batch(&batch)
}

/// An exact read of `SESSION` that carries `proofs`.
fn exact(
    store: &mut Store,
    records: &[CanonicalRecord],
    proofs: &[InjectedContextProof],
) -> IngestBatchOutcome {
    read(store, SESSION, NATIVE, records, &aligned(records, proofs)).unwrap()
}

/// The same read with no proof, as ordinary import submits it.
fn bulk(store: &mut Store, session: &str, native: &str, records: &[CanonicalRecord]) {
    let saved = read(store, session, native, records, &[]).unwrap();
    assert!(saved.injected_context.is_empty());
}

fn fresh(directory: &TempDir, name: &str) -> (std::path::PathBuf, Store) {
    let path = directory.path().join(format!("{name}.sqlite"));
    let store = Store::open(&path).unwrap();
    (path, store)
}

fn abstained(reason: InjectedContextAbstention) -> InjectedContextDisposition {
    InjectedContextDisposition::Abstained(reason)
}

/// The disposition of the one proof a batch carried.
fn only(outcome: &IngestBatchOutcome) -> InjectedContextDisposition {
    assert_eq!(outcome.injected_context.len(), 1, "{outcome:?}");
    outcome.injected_context[0].disposition
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

/// Everything that is not a proof: raw rows, usage, tools, sessions.
fn work(path: &std::path::Path) -> Vec<String> {
    let sql = Connection::open(path).unwrap();
    let mut rows = Vec::new();
    for query in [
        "SELECT json_array(uuid,session_id,type,ts,ts_ms,api_message_id,is_meta,is_sidechain,role,model,
             is_tool_result_carrier,text_len,tool_use_count,content_json,has_conflict,is_human,is_command)
         FROM records ORDER BY uuid",
        "SELECT json_array(uuid,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens) FROM usage ORDER BY uuid",
        "SELECT json_array(uuid,block_index,name,input_json,kind) FROM tool_uses ORDER BY uuid,block_index",
        "SELECT json_array(session_id,host,source,native_session_id,record_count,first_ts,last_ts,has_conflict)
         FROM sessions ORDER BY session_id",
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

fn scalar(path: &std::path::Path, query: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(query, [], |row| row.get(0))
        .unwrap()
}

fn proofs(path: &std::path::Path) -> i64 {
    scalar(path, "SELECT count(*) FROM injected_context_inputs")
}

#[test]
fn a_proof_committed_with_its_new_record_excludes_exactly_that_input() {
    let directory = TempDir::new().unwrap();
    let (path, mut store) = fresh(&directory, "proven");
    let saved = exact(&mut store, &records(), &[proof(SKILL)]);
    assert!(
        saved
            .records
            .iter()
            .all(|row| row.disposition == RecordDisposition::Inserted)
    );
    assert_eq!(
        saved.injected_context,
        [InjectedContextOutcome {
            input_index: 1,
            disposition: InjectedContextDisposition::Recorded,
        }]
    );

    // Only the proven input stops counting. The person's own request and the
    // pasted lookalike, whose text is identical to the body, are human.
    let corrected = projected(&path);
    assert_eq!(
        corrected,
        [
            (id(ASK), Some(1), Some(1), 0),
            (id(SKILL), Some(0), Some(1), 1),
            (id(WORK), Some(0), Some(0), 0),
            (id(RESULT), Some(0), Some(0), 0),
            (id(PASTED), Some(1), Some(1), 0),
        ]
    );
    // Every raw fact is exactly what the same read without the proof stores.
    let (plain_path, mut plain) = fresh(&directory, "plain");
    bulk(&mut plain, SESSION, NATIVE, &records());
    assert_eq!(work(&path), work(&plain_path));
    assert_eq!(
        projected(&plain_path)
            .into_iter()
            .filter(|row| row.3 == 1 || row.1 != row.2)
            .count(),
        0
    );
    for record in store.records(SESSION).unwrap() {
        assert_eq!(record.confirmed_automated_input, record.uuid == id(SKILL));
    }
    let typed = store.records(SESSION).unwrap();
    let body = typed.iter().find(|r| r.uuid == id(SKILL)).unwrap();
    assert_eq!(body.classification.is_human, Some(true));
    assert_eq!(
        store.injected_context_proofs(SESSION).unwrap(),
        [proof(SKILL)]
    );
    assert_eq!(
        scalar(&path, "SELECT observed_at FROM injected_context_inputs"),
        5_000
    );
    // Metadata only: no text is retained anywhere, the proof row included.
    assert_eq!(
        scalar(
            &path,
            "SELECT count(*) FROM records WHERE content_json IS NOT NULL"
        ),
        0
    );
    let stored: String = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT json_array(record_uuid,session_id,native_session_id,evidence_contract,
                 evidence_version,evidence_kind,segment_history,rollout_id,row_index,row_ordinal,
                 item_id,turn_id,observed_at) FROM injected_context_inputs",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!stored.contains("Synthetic"), "{stored}");

    // An identical retry of the exact read finds every record already held
    // and records nothing; an ordinary replay without the proof neither
    // restores the input nor duplicates anything.
    let retry = exact(&mut store, &records(), &[proof(SKILL)]);
    assert!(
        retry
            .records
            .iter()
            .all(|row| row.disposition == RecordDisposition::Duplicate)
    );
    assert_eq!(
        only(&retry),
        abstained(InjectedContextAbstention::NotInserted)
    );
    bulk(&mut store, SESSION, NATIVE, &records());
    assert_eq!(proofs(&path), 1);
    assert_eq!(projected(&path), corrected);
    drop(store);

    let reopened = Store::open(&path).unwrap();
    assert_eq!(projected(&path), corrected);
    assert_eq!(work(&path), work(&plain_path));
    assert_eq!(
        reopened.injected_context_proofs(SESSION).unwrap(),
        [proof(SKILL)]
    );
}

#[test]
fn a_proof_never_joins_a_record_the_index_already_held() {
    let directory = TempDir::new().unwrap();
    let not_inserted = abstained(InjectedContextAbstention::NotInserted);

    // Bulk import stored the input first; a later exact read cannot upgrade it.
    let (path, mut store) = fresh(&directory, "bulk-first");
    bulk(&mut store, SESSION, NATIVE, &records());
    let before = projected(&path);
    let later = exact(&mut store, &records(), &[proof(SKILL)]);
    assert_eq!(later.records[1].disposition, RecordDisposition::Duplicate);
    assert_eq!(only(&later), not_inserted);
    assert_eq!(projected(&path), before);
    assert!(before.contains(&(id(SKILL), Some(1), Some(1), 0)));
    assert_eq!(proofs(&path), 0);

    // The index held the input with unknown content; the exact read enriches
    // it into a human input, and still does not prove where it came from.
    let (path, mut store) = fresh(&directory, "enriched");
    bulk(
        &mut store,
        SESSION,
        NATIVE,
        &[record(
            json!({"uuid":id(SKILL),"type":"user","timestamp":"2026-09-07T12:00:01Z",
                "message":{"role":"user"}}),
        )],
    );
    let later = exact(&mut store, &records(), &[proof(SKILL)]);
    assert_eq!(later.records[1].disposition, RecordDisposition::Enriched);
    assert_eq!(only(&later), not_inserted);
    assert!(projected(&path).contains(&(id(SKILL), Some(1), Some(1), 0)));
    assert_eq!(proofs(&path), 0);

    // Another session owns the UUID (a copy of its history): the occurrence
    // here is rejected, and the owner's input is not touched.
    let (path, mut store) = fresh(&directory, "copied");
    bulk(&mut store, OTHER, OTHER_NATIVE, &[text(SKILL, 1, BODY)]);
    let later = exact(&mut store, &records(), &[proof(SKILL)]);
    assert_eq!(
        later.records[1].disposition,
        RecordDisposition::RejectedOwnership
    );
    assert_eq!(only(&later), not_inserted);
    assert_eq!(proofs(&path), 0);
    let owner = store.records(OTHER).unwrap();
    assert!(!owner[0].confirmed_automated_input);
    assert_eq!(owner[0].classification.is_human, Some(true));

    // The session already holds the UUID as another record type.
    let (path, mut store) = fresh(&directory, "retyped");
    bulk(
        &mut store,
        SESSION,
        NATIVE,
        &[record(
            json!({"uuid":id(SKILL),"type":"assistant","timestamp":"2026-09-07T12:00:01Z",
                "message":{"role":"assistant","content":[{"type":"text","text":"Synthetic"}]}}),
        )],
    );
    let later = exact(&mut store, &records(), &[proof(SKILL)]);
    assert_eq!(
        later.records[1].disposition,
        RecordDisposition::RejectedType
    );
    assert_eq!(only(&later), not_inserted);
    assert_eq!(proofs(&path), 0);

    // A new record whose own observation disagrees with itself is conflicted.
    let (path, mut store) = fresh(&directory, "conflicted");
    let mut rows = records();
    rows[1] = record(
        json!({"uuid":id(SKILL),"type":"user","timestamp":"2026-09-07T12:00:01Z",
            "api_message_id":"synthetic-a",
            "message":{"id":"synthetic-b","role":"user","content":[{"type":"text","text":BODY}]}}),
    );
    let saved = exact(&mut store, &rows, &[proof(SKILL)]);
    assert_eq!(saved.records[1].disposition, RecordDisposition::Inserted);
    assert_eq!(saved.records[1].stored_has_conflict, Some(true));
    assert_eq!(
        only(&saved),
        abstained(InjectedContextAbstention::ConflictedRecord)
    );
    assert_eq!(proofs(&path), 0);

    // A second occurrence of the proven UUID in the same read makes the
    // record's outcome ambiguous: the whole batch is refused.
    let (path, mut store) = fresh(&directory, "repeated");
    let mut rows = records();
    rows.push(text(SKILL, 1, BODY));
    let mut claims = aligned(&records(), &[proof(SKILL)]);
    claims.push(None);
    assert!(read(&mut store, SESSION, NATIVE, &rows, &claims).is_err());
    assert_eq!(store.counts().unwrap().sessions, 0);
    assert_eq!(store.counts().unwrap().records, 0);
    assert!(store.discovered_sessions(Host::Codex).unwrap().is_empty());
    assert_eq!(proofs(&path), 0);
}

#[test]
fn a_new_input_the_confirmation_rules_could_not_correct_abstains() {
    let directory = TempDir::new().unwrap();
    let ineligible = |reason| abstained(InjectedContextAbstention::Ineligible(reason));
    let extra = |mut value: serde_json::Value| {
        value["uuid"] = json!(id(NEW));
        value["timestamp"] = json!("2026-09-07T12:00:05Z");
        record(value)
    };
    let cases = [
        (
            "meta",
            extra(json!({"type":"user","isMeta":true,
                "message":{"role":"user","content":[{"type":"text","text":BODY}]}})),
            ineligible(Abstention::NotUserInput),
        ),
        (
            "sidechain",
            extra(json!({"type":"user","isSidechain":true,
                "message":{"role":"user","content":[{"type":"text","text":BODY}]}})),
            ineligible(Abstention::NotUserInput),
        ),
        (
            "tool-result",
            extra(json!({"type":"user",
                "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"synthetic"}]}})),
            ineligible(Abstention::NotUserInput),
        ),
        (
            "mixed",
            extra(json!({"type":"user","message":{"role":"user","content":[
                {"type":"text","text":BODY},
                {"type":"tool_result","tool_use_id":"t","content":"synthetic"}]}})),
            ineligible(Abstention::NotUserInput),
        ),
        (
            "assistant",
            extra(json!({"type":"assistant",
                "message":{"role":"assistant","content":[{"type":"text","text":BODY}]}})),
            ineligible(Abstention::NotUserInput),
        ),
        (
            "command",
            extra(json!({"type":"user",
                "message":{"role":"user","content":[{"type":"text","text":"<command-name>/synthetic</command-name>"}]}})),
            ineligible(Abstention::NotHumanClassified),
        ),
        (
            "unknown",
            extra(json!({"type":"user","message":{"role":"user"}})),
            ineligible(Abstention::NotHumanClassified),
        ),
    ];
    for (name, input, expected) in cases {
        let (path, mut store) = fresh(&directory, name);
        let mut rows = records();
        rows.push(input);
        let saved = exact(&mut store, &rows, &[proof(NEW)]);
        assert_eq!(saved.records[5].disposition, RecordDisposition::Inserted);
        assert_eq!(only(&saved), expected, "{name}");
        assert_eq!(proofs(&path), 0, "{name}");
        // Exactly what the read without the proof stores and projects.
        let (plain_path, mut plain) = fresh(&directory, &format!("{name}-plain"));
        bulk(&mut plain, SESSION, NATIVE, &rows);
        assert_eq!(work(&path), work(&plain_path), "{name}");
        assert_eq!(projected(&path), projected(&plain_path), "{name}");
    }

    // The session as the index already held it decides, not the batch's
    // description of it: another source, another native identity, a judge.
    type Prepared = (&'static str, fn(&Connection), InjectedContextDisposition);
    let prepared: [Prepared; 3] = [
        (
            "plugin",
            |_| {},
            ineligible(Abstention::NotCodexReaderHistory),
        ),
        (
            "native",
            |_| {},
            ineligible(Abstention::NativeIdentityMismatch),
        ),
        (
            "judge",
            |sql| {
                sql.execute("UPDATE sessions SET kind='judge'", []).unwrap();
            },
            ineligible(Abstention::NotUserInput),
        ),
    ];
    for (name, prepare, expected) in prepared {
        let (path, mut store) = fresh(&directory, &format!("session-{name}"));
        let mut existing = match name {
            "plugin" => SessionMeta::new(SESSION, "codex", SessionSource::Plugin),
            _ => meta(SESSION, NATIVE),
        };
        let native = if name == "native" {
            OTHER_NATIVE
        } else {
            NATIVE
        };
        existing.native_session_id = Some(native.into());
        store.upsert_session(&existing, false).unwrap();
        prepare(&Connection::open(&path).unwrap());
        let saved = exact(&mut store, &records(), &[proof(SKILL)]);
        assert_eq!(saved.records[1].disposition, RecordDisposition::Inserted);
        assert_eq!(only(&saved), expected, "{name}");
        assert_eq!(proofs(&path), 0, "{name}");
    }
}

#[test]
fn malformed_misbound_or_ambiguous_proofs_reject_the_whole_batch() {
    let directory = TempDir::new().unwrap();
    let (path, mut store) = fresh(&directory, "refused");
    let rows = records();
    let refused = |store: &mut Store, claims: &[Option<InjectedContextProof>]| {
        assert!(
            read(store, SESSION, NATIVE, &rows, claims).is_err(),
            "{claims:?}"
        );
    };
    let with = |change: &dyn Fn(&mut InjectedContextProof)| {
        let mut broken = proof(SKILL);
        change(&mut broken);
        let mut claims = vec![None; rows.len()];
        claims[1] = Some(broken);
        claims
    };
    let changes: &[&dyn Fn(&mut InjectedContextProof)] = &[
        // Closed version and field shapes.
        &|p| p.version = 2,
        &|p| p.version = 0,
        &|p| p.item_id = item(SKILL).to_uppercase(),
        &|p| p.item_id = format!("rs_{}", "a".repeat(50)),
        &|p| p.item_id = format!("msg_{}", "a".repeat(49)),
        &|p| p.item_id = format!("msg_{}", "a".repeat(51)),
        &|p| p.item_id = "msg_".into(),
        &|p| p.turn_id = "1".into(),
        &|p| p.turn_id = turn(1).to_uppercase(),
        &|p| p.turn_id = format!("{} ", turn(1)),
        // A segment must say what kind of rollout it is.
        &|p| p.rollout_id = Some(ROLLOUT.into()),
        &|p| p.history = SegmentHistory::Paginated,
        &|p| {
            p.history = SegmentHistory::Paginated;
            p.rollout_id = Some("rollout".into());
        },
        // Another owner, native session or record than the input it is beside.
        &|p| p.native_session_id = OTHER_NATIVE.into(),
        &|p| p.native_session_id = NATIVE.to_uppercase(),
        &|p| p.record_uuid = id(ASK),
        &|p| p.record_uuid = id(99),
    ];
    for change in changes {
        refused(&mut store, &with(*change));
    }
    // Not aligned with the inputs.
    refused(&mut store, &[Some(proof(SKILL))]);
    let mut long = aligned(&rows, &[proof(SKILL)]);
    long.push(None);
    refused(&mut store, &long);
    // One item or one native row named for two inputs.
    let same_item = InjectedContextProof {
        item_id: item(SKILL),
        ..proof(PASTED)
    };
    refused(&mut store, &aligned(&rows, &[proof(SKILL), same_item]));
    let same_row = InjectedContextProof {
        row_index: proof(SKILL).row_index,
        ..proof(PASTED)
    };
    refused(&mut store, &aligned(&rows, &[proof(SKILL), same_row]));
    // A record UUID the reader could not have emitted is no canonical UUID.
    let mut named = rows.clone();
    named[1].uuid = Some("skill".into());
    let odd = InjectedContextProof {
        record_uuid: "skill".into(),
        ..proof(SKILL)
    };
    assert!(
        read(
            &mut store,
            SESSION,
            NATIVE,
            &named,
            &aligned(&named, &[odd])
        )
        .is_err()
    );
    // The proven session is another one than the batch's.
    assert!(
        read(
            &mut store,
            OTHER,
            OTHER_NATIVE,
            &rows,
            &aligned(&rows, &[proof(SKILL)])
        )
        .is_err()
    );

    // Ordinary import cannot carry a proof: not as Codex bulk, not as a
    // plugin capture, not as Claude native history.
    let claims = aligned(&rows, &[proof(SKILL)]);
    let codex = meta(SESSION, NATIVE);
    let mut batch = IngestBatch::new(&codex, &rows, false);
    batch.injected_context = &claims;
    assert!(store.apply_ingest_batch(&batch).is_err());
    let mut plugin = SessionMeta::new(SESSION, "codex", SessionSource::Plugin);
    plugin.native_session_id = Some(NATIVE.into());
    let mut batch = IngestBatch::new(&plugin, &rows, false);
    batch.injected_context = &claims;
    assert!(store.apply_ingest_batch(&batch).is_err());
    let mut claude = SessionMeta::new(SESSION, "claude", SessionSource::Transcript);
    claude.native_session_id = Some(NATIVE.into());
    let discovery = DiscoveredSession {
        host: Host::Claude,
        native_session_id: NATIVE.into(),
        conversation_id: Some(SESSION.into()),
        surface: None,
        started_at_ms: None,
        last_observed_at: 5_000,
        discovery_complete: true,
    };
    let mut batch = IngestBatch::new(&claude, &rows, false);
    batch.native_history = true;
    batch.discovery = Some(&discovery);
    batch.injected_context = &claims;
    assert!(store.apply_ingest_batch(&batch).is_err());

    // Nothing of any refused batch was written.
    assert_eq!(store.counts().unwrap().sessions, 0);
    assert_eq!(store.counts().unwrap().records, 0);
    assert!(store.discovered_sessions(Host::Codex).unwrap().is_empty());
    assert!(store.discovered_sessions(Host::Claude).unwrap().is_empty());
    assert_eq!(proofs(&path), 0);

    // The accepted shapes: a paginated continuation with a native ordinal
    // and a 50-hex item ID, and a flat rollout's UUID item ID.
    let paginated = InjectedContextProof {
        history: SegmentHistory::Paginated,
        rollout_id: Some(ROLLOUT.into()),
        row_ordinal: Some(7),
        item_id: "msg_0123456789abcdef0123456789abcdef0123456789abcdef01".into(),
        ..proof(SKILL)
    };
    let saved = exact(&mut store, &rows, &[paginated.clone(), proof(PASTED)]);
    assert_eq!(
        saved
            .injected_context
            .iter()
            .map(|outcome| (outcome.input_index, outcome.disposition))
            .collect::<Vec<_>>(),
        [
            (1, InjectedContextDisposition::Recorded),
            (4, InjectedContextDisposition::Recorded)
        ]
    );
    assert_eq!(
        store.injected_context_proofs(SESSION).unwrap(),
        [paginated, proof(PASTED)]
    );
}

#[test]
fn an_input_holds_one_proof_across_all_three_tables() {
    let directory = TempDir::new().unwrap();
    let (path, mut store) = fresh(&directory, "tables");
    exact(&mut store, &records(), &[proof(SKILL)]);
    let corrected = projected(&path);

    // Neither confirmation path can claim an input a proof holds.
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
    let guardian = |n: u32| GuardianTurnProof {
        record_uuid: id(n),
        session_id: SESSION.into(),
        native_session_id: NATIVE.into(),
        turn_id: turn(n),
        parent_native_session_id: PARENT.into(),
        parent_turn_id: format!("abcdef00-0000-7000-b000-{n:012x}"),
        evidence_kind: GuardianEvidenceKind::GuardianTurnDispatch,
        matcher_version: 1,
    };
    let conflicting = [ConfirmationDisposition::Abstained(
        Abstention::ConflictingConfirmation,
    )];
    assert_eq!(
        store
            .apply_automated_input_confirmations(&[dispatch(SKILL, "call-one")], 2_000)
            .unwrap()
            .dispositions,
        conflicting
    );
    assert_eq!(
        store
            .apply_guardian_turn_confirmations(&[guardian(SKILL)], 2_000)
            .unwrap()
            .dispositions,
        conflicting
    );
    assert_eq!(
        scalar(&path, "SELECT count(*) FROM confirmed_automated_inputs"),
        0
    );
    assert_eq!(
        scalar(&path, "SELECT count(*) FROM guardian_turn_inputs"),
        0
    );
    assert_eq!(projected(&path), corrected);

    // The schema refuses a proof for an input either confirmation holds.
    assert_eq!(
        store
            .apply_guardian_turn_confirmations(&[guardian(PASTED)], 2_000)
            .unwrap()
            .dispositions,
        [ConfirmationDisposition::Confirmed]
    );
    assert_eq!(
        store
            .apply_automated_input_confirmations(&[dispatch(ASK, "call-two")], 2_000)
            .unwrap()
            .dispositions,
        [ConfirmationDisposition::Confirmed]
    );
    let sql = Connection::open(&path).unwrap();
    for (n, row) in [(PASTED, 40), (ASK, 41)] {
        assert!(
            sql.execute(
                "INSERT INTO injected_context_inputs VALUES(?1,?2,?3,'memhub.codex.origin_evidence',1,
                     'codex_selected_skill_instructions','flat',NULL,?4,NULL,?5,?6,1)",
                rusqlite::params![id(n), SESSION, NATIVE, row, item(row), turn(1)],
            )
            .is_err()
        );
    }
    assert_eq!(proofs(&path), 1);

    // A later read's new record cannot reuse a stored proof's native item or
    // native row: it is stored and stays human.
    let reused_item = InjectedContextProof {
        row_index: 50,
        item_id: item(SKILL),
        ..proof(NEW)
    };
    let reused_row = InjectedContextProof {
        row_index: proof(SKILL).row_index,
        item_id: item(51),
        ..proof(NEW + 1)
    };
    for (n, reused) in [(NEW, reused_item), (NEW + 1, reused_row)] {
        let saved = exact(&mut store, &[text(n, 30, BODY)], &[reused]);
        assert_eq!(saved.records[0].disposition, RecordDisposition::Inserted);
        assert_eq!(
            only(&saved),
            abstained(InjectedContextAbstention::ConflictingProof)
        );
        assert!(projected(&path).contains(&(id(n), Some(1), Some(1), 0)));
    }
    assert_eq!(
        store.injected_context_proofs(SESSION).unwrap(),
        [proof(SKILL)]
    );
}

#[test]
fn the_table_holds_only_closed_structural_identities_and_is_immutable() {
    let directory = TempDir::new().unwrap();
    let (path, mut store) = fresh(&directory, "schema");
    exact(&mut store, &records(), &[proof(SKILL)]);
    bulk(&mut store, OTHER, OTHER_NATIVE, &[text(NEW, 0, BODY)]);
    let sql = Connection::open(&path).unwrap();
    let columns: Vec<(String, String)> = sql
        .prepare("SELECT name,type FROM pragma_table_info('injected_context_inputs') ORDER BY cid")
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
            ("evidence_contract", "TEXT"),
            ("evidence_version", "INTEGER"),
            ("evidence_kind", "TEXT"),
            ("segment_history", "TEXT"),
            ("rollout_id", "TEXT"),
            ("row_index", "INTEGER"),
            ("row_ordinal", "INTEGER"),
            ("item_id", "TEXT"),
            ("turn_id", "TEXT"),
            ("observed_at", "INTEGER"),
        ]
        .map(|(name, kind)| (name.to_owned(), kind.to_owned()))
    );
    for statement in [
        "UPDATE injected_context_inputs SET row_index=row_index+1",
        "UPDATE injected_context_inputs SET evidence_kind=evidence_kind",
        "DELETE FROM injected_context_inputs",
    ] {
        assert!(sql.execute(statement, []).is_err(), "{statement}");
    }

    // Direct writes must still have the closed shape. Start from a valid row
    // for the pasted input and break one fact at a time.
    let valid: [SqlValue; 13] = [
        id(PASTED).into(),
        SESSION.to_owned().into(),
        NATIVE.to_owned().into(),
        "memhub.codex.origin_evidence".to_owned().into(),
        1_i64.into(),
        "codex_selected_skill_instructions".to_owned().into(),
        "flat".to_owned().into(),
        SqlValue::Null,
        30_i64.into(),
        SqlValue::Null,
        item(30).into(),
        turn(1).into(),
        1_i64.into(),
    ];
    let insert = |values: &[SqlValue; 13]| {
        sql.execute(
            "INSERT INTO injected_context_inputs VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            rusqlite::params_from_iter(values.iter()),
        )
    };
    let string = |value: &str| SqlValue::from(value.to_owned());
    // Integer columns have integer affinity, so only text SQLite cannot read
    // as a number stays text for the type checks to refuse.
    let broken: [(usize, SqlValue); 21] = [
        (0, string(&id(PASTED).to_uppercase())),
        (1, string(NATIVE)),
        (1, string(OTHER)),
        (2, string(OTHER_NATIVE)),
        (3, string("memhub.codex.origin")),
        (4, SqlValue::Integer(2)),
        (4, string("one")),
        (5, string("agent_dispatch")),
        (6, string("legacy")),
        (7, string(ROLLOUT)),
        (8, SqlValue::Integer(-1)),
        (8, SqlValue::Real(30.5)),
        (9, SqlValue::Integer(-1)),
        (10, string(&item(30).to_uppercase())),
        (10, string(&format!("msg_{}", "a".repeat(49)))),
        (10, string("item")),
        (11, string("1")),
        (12, string("now")),
        // A record in another session cannot be named.
        (0, string(&id(NEW))),
        // One native item, and one native row, name one input.
        (10, string(&item(SKILL))),
        (8, SqlValue::Integer(i64::from(proof(SKILL).row_index))),
    ];
    for (index, value) in broken {
        let mut values = valid.clone();
        values[index] = value;
        assert!(insert(&values).is_err(), "{values:?}");
    }
    // A paginated row needs its rollout.
    let mut paginated = valid.clone();
    paginated[6] = string("paginated");
    assert!(insert(&paginated).is_err());
    assert_eq!(
        store.injected_context_proofs(SESSION).unwrap(),
        [proof(SKILL)]
    );
    // Each refusal above was its one broken fact: the unbroken row is legal,
    // and so is its paginated form. No API writes either; only SQL can.
    paginated[7] = string(ROLLOUT);
    paginated[0] = id(ASK).into();
    paginated[10] = item(31).into();
    assert_eq!(insert(&valid).unwrap(), 1);
    assert_eq!(insert(&paginated).unwrap(), 1);
    assert_eq!(proofs(&path), 3);
}
