//! An injected context proof marks one Codex input as the instructions Codex
//! added for a selected skill. It inherits the confirmed-input semantics
//! exactly: that one input stops counting as human, the person's own request
//! beside it and pasted text that looks like it still count, and no agent
//! work, token or all-event measurement moves. A schema 12 index upgrades to it
//! without touching its Guardian or tool-call confirmations or any metric.

use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use xt_fixtures::TempDb;
use xt_metrics::{MetricsDb, TypingRate, Window};
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    batch::IngestBatch,
    confirmation::{
        AutomatedInputProof, ConfirmationDisposition, EvidenceKind, GuardianEvidenceKind,
        GuardianTurnProof,
    },
    ingest::DiscoveredSession,
    injected::{
        InjectedContextDisposition, InjectedContextKind, InjectedContextProof, OriginContract,
        SegmentHistory,
    },
};

// Synthetic canonical identities; none names a real session, item or turn.
const NATIVE: &str = "abcdef00-0000-7000-8000-00000000eeee";
const SKILLED: &str = "codex-abcdef00-0000-7000-8000-00000000eeee";
const GUARDIAN_NATIVE: &str = "abcdef00-0000-7000-8000-00000000aaaa";
const GUARDIAN: &str = "codex-abcdef00-0000-7000-8000-00000000aaaa";
const PARENT: &str = "abcdef00-0000-7000-8000-00000000dddd";
const DISPATCHED: &str = "synthetic-dispatched";
const GUARDIAN_INPUTS: u32 = 621;
const DISPATCHED_INPUTS: u32 = 8;

/// The `v_records` definition an installed schema 12 index holds.
const SCHEMA_12_RECORDS_VIEW: &str = "CREATE VIEW v_records AS
SELECT r.uuid,r.session_id,r.type,r.ts,r.ts_ms,r.api_message_id,r.request_id,
       r.is_sidechain,r.role,r.model,r.is_tool_result_carrier,r.text_len,
       r.tool_use_count,
       -- Effective M-02 eligibility: a structurally confirmed automated input,
       -- from either confirmation table, is not a human message. raw_is_human
       -- keeps ingestion's classification.
       CASE WHEN a.record_uuid IS NULL AND g.record_uuid IS NULL THEN r.is_human ELSE 0 END
           AS is_human,
       r.is_human AS raw_is_human,
       (a.record_uuid IS NOT NULL OR g.record_uuid IS NOT NULL) AS confirmed_automated_input,
       r.is_command,r.is_interrupted,r.is_system_reminder,
       r.parent_uuid,r.agent_id,r.subtype,r.has_conflict,
       s.host,s.source_platform,s.surface,s.source,s.kind,
       u.input_tokens,u.output_tokens,u.cache_read_tokens,u.cache_creation_tokens,
       u.cache_creation_5m,u.cache_creation_1h,u.service_tier,u.uuid IS NOT NULL AS usage_observed
FROM records r JOIN sessions s ON s.session_id=r.session_id
LEFT JOIN usage u ON u.uuid=r.uuid
LEFT JOIN confirmed_automated_inputs a
    ON a.record_uuid=r.uuid AND a.session_id=r.session_id AND r.is_human=1
LEFT JOIN guardian_turn_inputs g
    ON g.record_uuid=r.uuid AND g.session_id=r.session_id AND r.is_human=1
WHERE r.is_meta=0 AND s.kind='user' AND (r.model IS NULL OR r.model<>'<synthetic>');";

fn ms(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
/// An instant on 2026-09-07.
fn at(hour: u32, minute: u32, second: u32) -> String {
    format!("2026-09-07T{hour:02}:{minute:02}:{second:02}Z")
}
/// An instant `seconds` after 2026-09-06T00:00:00Z.
fn after_start(seconds: u32) -> String {
    Timestamp::from_second(ms("2026-09-06T00:00:00Z") / 1_000 + i64::from(seconds))
        .unwrap()
        .to_string()
}
/// A synthetic canonical record UUID ending in `n`, for the reviewer session.
fn id(n: u32) -> String {
    format!("abcdef00-0000-5000-8000-{n:012x}")
}
/// A synthetic canonical record UUID ending in `n`, for the skilled session.
fn sid(n: u32) -> String {
    format!("abcdef00-0000-5000-9000-{n:012x}")
}
fn turn(n: u32) -> String {
    format!("abcdef00-0000-7000-9000-{n:012x}")
}
fn record(value: Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn input(uuid: &str, ts: &str, text: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user","timestamp":ts,
        "message":{"role":"user","content":[{"type":"text","text":text}]}}))
}
fn tool(uuid: &str, ts: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"assistant","timestamp":ts,
        "message":{"id":format!("msg-{uuid}"),"role":"assistant","model":"synthetic-model",
            "content":[{"type":"tool_use","name":"Read","input":{"path":format!("synthetic/{uuid}")}}],
            "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":1,"cache_creation_input_tokens":2}}}))
}
fn result(uuid: &str, ts: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user","timestamp":ts,
        "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"synthetic"}]}}))
}
fn reply(uuid: &str, ts: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"assistant","timestamp":ts,
        "message":{"id":format!("msg-{uuid}"),"role":"assistant","model":"synthetic-model",
            "content":[{"type":"text","text":"Synthetic reply"}],
            "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":1,"cache_creation_input_tokens":2}}}))
}

const ASK: u32 = 1;
const BODY: u32 = 2;
const PASTED: u32 = 6;

/// One Codex session read exactly: the person's 50-character `$skill`
/// request, the 400-character body Codex injected a second later, the work
/// they started, and an hour later 150 characters a person pasted that read
/// like a skill body, answered by one reply.
fn skilled_rows() -> Vec<CanonicalRecord> {
    vec![
        input(
            &sid(ASK),
            &at(9, 0, 0),
            &format!("$synthetic-skill {}", "a".repeat(33)),
        ),
        input(&sid(BODY), &at(9, 0, 1), &"x".repeat(400)),
        tool(&sid(3), &at(9, 1, 0)),
        result(&sid(4), &at(9, 2, 0)),
        reply(&sid(5), &at(9, 3, 0)),
        input(&sid(PASTED), &at(10, 0, 0), &"x".repeat(150)),
        reply(&sid(7), &at(10, 1, 0)),
    ]
}

fn body_proof() -> InjectedContextProof {
    InjectedContextProof {
        contract: OriginContract::CodexOriginEvidence,
        version: 1,
        kind: InjectedContextKind::CodexSelectedSkillInstructions,
        native_session_id: NATIVE.into(),
        history: SegmentHistory::Flat,
        rollout_id: None,
        row_index: 7,
        row_ordinal: None,
        item_id: format!("msg_{}", turn(99)),
        turn_id: turn(1),
        record_uuid: sid(BODY),
    }
}

/// Read the skilled session as the native Codex reader's importer does, with
/// the body's proof when `proven`.
fn read_skilled(store: &mut Store, proven: bool) {
    let mut meta = SessionMeta::new(SKILLED, "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some(NATIVE.into());
    meta.surface = Some("cli".into());
    let discovery = DiscoveredSession {
        host: Host::Codex,
        native_session_id: NATIVE.into(),
        conversation_id: Some(SKILLED.into()),
        surface: Some("cli".into()),
        started_at_ms: None,
        last_observed_at: 5_000,
        discovery_complete: true,
    };
    let rows = skilled_rows();
    let claims = rows
        .iter()
        .map(|row| (proven && row.uuid == Some(sid(BODY))).then(body_proof))
        .collect::<Vec<_>>();
    let mut batch = IngestBatch::new(&meta, &rows, false);
    batch.native_codex = true;
    batch.discovery = Some(&discovery);
    if proven {
        batch.injected_context = &claims;
    }
    let saved = store.apply_ingest_batch(&batch).unwrap();
    let recorded = saved
        .injected_context
        .iter()
        .map(|outcome| outcome.disposition)
        .collect::<Vec<_>>();
    if proven {
        assert_eq!(recorded, [InjectedContextDisposition::Recorded]);
    } else {
        assert!(recorded.is_empty());
    }
}

fn skilled(proven: bool) -> TempDb {
    let mut db = TempDb::empty().unwrap();
    read_skilled(db.store_mut(), proven);
    db
}

/// Every dashboard and session measurement this change could reach.
fn measured(path: &std::path::Path, ids: &[&str]) -> Value {
    let m = MetricsDb::open(path).unwrap();
    let rate = TypingRate::default();
    json!({
        "counts": m.counts(window(), rate).unwrap(),
        "human": m.human_time(window(), rate, TimeZone::UTC).unwrap(),
        "tokens": m.tokens(window(), TimeZone::UTC).unwrap(),
        "spans": m.active_spans(window()).unwrap(),
        "concurrency": m.concurrency(window()).unwrap(),
        "hands_off": m.hands_off(window()).unwrap(),
        "session_hands_off": m.session_hands_off(window(), ids).unwrap(),
        "windows": m.session_windows(window(), ids).unwrap(),
        "favorite": m.favorite_model(window()).unwrap(),
        "stretches": ids.iter().map(|id| m.session_stretches(window(), id).unwrap()).collect::<Vec<_>>(),
    })
}

/// `(uuid, effective is_human, raw is_human, confirmed)` for every record.
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

#[test]
fn one_injected_skill_body_moves_only_its_input_s_human_measurements() {
    let (unproven, proven) = (skilled(false), skilled(true));
    let ids = [SKILLED];
    let (before, after) = (
        measured(unproven.path(), &ids),
        measured(proven.path(), &ids),
    );
    let count = |m: &Value, field: &str| m["counts"][field].clone();

    // M-02: one fewer human message and exactly its 400 characters fewer. The
    // person's request and the pasted lookalike still count.
    assert_eq!(count(&before, "human_messages"), 3);
    assert_eq!(count(&after, "human_messages"), 2);
    assert_eq!(count(&before, "human_chars"), 600);
    assert_eq!(count(&after, "human_chars"), 200);
    assert_eq!(count(&before, "typing_minutes_est"), 3.0);
    assert_eq!(count(&after, "typing_minutes_est"), 1.0);
    // M-03: the request already opened the turn the work answered, and the
    // pasted text opens the second; the body opened neither.
    assert_eq!(count(&before, "assistant_turns"), 2);
    assert_eq!(count(&after, "assistant_turns"), 2);
    // Human minutes use the same retained character total as counts.
    assert_eq!(before["human"]["human_minutes_est"], 3.0);
    assert_eq!(after["human"]["human_minutes_est"], 1.0);
    // Per session: only the human message count moves.
    let windows = |m: &Value| m["windows"][SKILLED].clone();
    assert_eq!(windows(&before)["human_messages"], 3);
    assert_eq!(windows(&after)["human_messages"], 2);
    for field in ["events", "tool_calls", "tokens", "agent_ms"] {
        assert_eq!(windows(&before)[field], windows(&after)[field], "{field}");
    }
    // M-09: the body never starts a stretch of its own.
    let starts = |m: &Value| {
        m["stretches"][0]["stretches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["start_uuid"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert!(!starts(&after).contains(&sid(BODY)), "{:?}", starts(&after));
    // Agent work, tokens and all-event measurements are identical.
    for field in ["tokens", "spans", "concurrency"] {
        assert_eq!(before[field], after[field], "{field}");
    }
    for field in ["sessions", "assistant_records", "tool_calls"] {
        assert_eq!(count(&before, field), count(&after, field), "{field}");
    }
    assert_eq!(
        before["human"]["agent_minutes"],
        after["human"]["agent_minutes"]
    );
    // Raw classification is ingestion's on both sides.
    let raw = |path: &std::path::Path| {
        projected(path)
            .into_iter()
            .map(|(uuid, _, raw, _)| (uuid, raw))
            .collect::<Vec<_>>()
    };
    assert_eq!(raw(unproven.path()), raw(proven.path()));
}

/// Eight tool-call confirmations in a Claude session, and 621 Guardian turn
/// confirmations in one Codex reviewer session, as an installed index holds.
fn installed() -> TempDb {
    let mut db = TempDb::empty().unwrap();
    let store = db.store_mut();
    let mut meta = SessionMeta::new(GUARDIAN, "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some(GUARDIAN_NATIVE.into());
    meta.surface = Some("cli".into());
    store.upsert_session(&meta, false).unwrap();
    let reviews = (1..=GUARDIAN_INPUTS)
        .flat_map(|n| {
            [
                input(&id(n), &after_start(120 * n), "Synthetic reviewer request"),
                reply(&id(10_000 + n), &after_start(120 * n + 60)),
            ]
        })
        .collect::<Vec<_>>();
    store.upsert_records(GUARDIAN, &reviews, false).unwrap();
    let proofs = (1..=GUARDIAN_INPUTS)
        .map(|n| GuardianTurnProof {
            record_uuid: id(n),
            session_id: GUARDIAN.into(),
            native_session_id: GUARDIAN_NATIVE.into(),
            turn_id: turn(n),
            parent_native_session_id: PARENT.into(),
            parent_turn_id: format!("abcdef00-0000-7000-a000-{n:012x}"),
            evidence_kind: GuardianEvidenceKind::GuardianTurnDispatch,
            matcher_version: 1,
        })
        .collect::<Vec<_>>();
    let report = store
        .apply_guardian_turn_confirmations(&proofs, 1_000)
        .unwrap();
    assert!(
        report
            .dispositions
            .iter()
            .all(|d| *d == ConfirmationDisposition::Confirmed)
    );

    let mut meta = SessionMeta::new(DISPATCHED, "claude", SessionSource::Transcript);
    meta.native_session_id = Some(DISPATCHED.into());
    meta.surface = Some("cli".into());
    store.upsert_session(&meta, false).unwrap();
    let mut rows = vec![input("human", &at(12, 0, 0), "Synthetic human")];
    for n in 0..DISPATCHED_INPUTS {
        rows.push(input(
            &format!("dispatched-{n}"),
            &at(12, 2 * n + 1, 0),
            "Synthetic dispatched",
        ));
        rows.push(reply(&format!("reply-{n}"), &at(12, 2 * n + 2, 0)));
    }
    store.upsert_records(DISPATCHED, &rows, false).unwrap();
    let proofs = (0..DISPATCHED_INPUTS)
        .map(|n| AutomatedInputProof {
            record_uuid: format!("dispatched-{n}"),
            session_id: DISPATCHED.into(),
            native_session_id: DISPATCHED.into(),
            parent_host: Host::Codex,
            parent_session_id: "codex-synthetic-parent".into(),
            parent_tool_call_id: format!("call-{n}"),
            parent_operation_index: 0,
            parent_result_id: Some(format!("call-{n}-output")),
            evidence_kind: EvidenceKind::AgentDispatch,
            matcher_version: 1,
        })
        .collect::<Vec<_>>();
    let report = store
        .apply_automated_input_confirmations(&proofs, 1_000)
        .unwrap();
    assert_eq!(report.dispositions, [ConfirmationDisposition::Confirmed; 8]);
    db
}

/// Leave exactly what a schema 12 build wrote: no migration 13, no injected
/// context table, and that build's record projection.
fn rewind_to_schema_12(path: &std::path::Path) {
    let sql = Connection::open(path).unwrap();
    sql.execute_batch(&format!(
        "DELETE FROM schema_version WHERE version>=13;
         DROP VIEW v_response_usage; DROP VIEW v_usage_records;
         DROP VIEW v_session_events; DROP VIEW v_records;
         DROP TABLE injected_context_inputs; DROP TABLE IF EXISTS task_notification_inputs; DROP TABLE IF EXISTS record_previews; DROP TABLE IF EXISTS human_input_adjustments; DROP TABLE IF EXISTS human_session_origins; DROP TABLE claude_launch_groups; DROP TABLE claude_launch_group_members; DROP TABLE claude_launch_candidates; DROP TABLE claude_launch_staged_candidates;
         {SCHEMA_12_RECORDS_VIEW}
         {}; {}; {};",
        include_str!("../../xt-store/views/usage_records.sql"),
        include_str!("../../xt-store/views/response_usage.sql"),
        include_str!("../../xt-store/views/session_events.sql"),
    ))
    .unwrap();
}

/// Both confirmation tables exactly as stored: rowid and every column's type
/// and value.
fn confirmations(path: &std::path::Path) -> (Vec<String>, Vec<String>) {
    let sql = Connection::open(path).unwrap();
    let rows = |query: &str| -> Vec<String> {
        sql.prepare(query)
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    (
        rows(
            "SELECT rowid||'|'||quote(record_uuid)||'|'||quote(session_id)||'|'||
                 quote(native_session_id)||'|'||quote(turn_id)||'|'||
                 quote(parent_native_session_id)||'|'||quote(parent_turn_id)||'|'||
                 quote(evidence_kind)||'|'||quote(matcher_version)||'|'||quote(confirmed_at)
             FROM guardian_turn_inputs ORDER BY rowid",
        ),
        rows(
            "SELECT rowid||'|'||quote(record_uuid)||'|'||quote(session_id)||'|'||
                 quote(native_session_id)||'|'||quote(evidence_kind)||'|'||quote(matcher_version)||'|'||
                 quote(parent_host)||'|'||quote(parent_session_id)||'|'||quote(parent_tool_call_id)||'|'||
                 quote(parent_operation_index)||'|'||quote(parent_result_id)||'|'||quote(confirmed_at)
             FROM confirmed_automated_inputs ORDER BY rowid",
        ),
    )
}

fn scalar(path: &std::path::Path, query: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(query, [], |row| row.get(0))
        .unwrap()
}

fn records_view(path: &std::path::Path) -> String {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='view' AND name='v_records'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn table_exists(path: &std::path::Path, name: &str) -> bool {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
            [name],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn a_schema_12_index_upgrades_with_its_621_and_8_confirmations_and_metrics_unchanged() {
    let db = installed();
    let path = db.path().to_owned();
    let ids = [GUARDIAN, DISPATCHED];
    let old_metrics = measured(&path, &ids);
    rewind_to_schema_12(&path);
    assert!(MetricsDb::open(&path).is_err());
    assert!(!table_exists(&path, "injected_context_inputs"));
    assert_eq!(
        records_view(&path),
        SCHEMA_12_RECORDS_VIEW.trim_end_matches(';')
    );
    let old_rows = confirmations(&path);
    assert_eq!(old_rows.0.len(), 621);
    assert_eq!(old_rows.1.len(), 8);
    let old_projection = projected(&path);
    // The installed confirmations are already neutral: 621 + 8 inputs.
    assert_eq!(
        old_projection
            .iter()
            .filter(|row| row.3 == 1 && row.1 == Some(0) && row.2 == Some(1))
            .count(),
        629
    );
    let content = "SELECT (SELECT count(*) FROM records WHERE content_json IS NOT NULL)
        + (SELECT count(*) FROM tool_uses WHERE input_json IS NOT NULL)
        + (SELECT count(*) FROM sessions WHERE title IS NOT NULL)";
    assert_eq!(scalar(&path, content), 0);

    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 17);
        assert!(store.injected_context_proofs(GUARDIAN).unwrap().is_empty());
        drop(store);
        assert!(
            records_view(&path).contains("injected_context_inputs"),
            "view rebuilt on open"
        );
        assert_eq!(confirmations(&path), old_rows);
        assert_eq!(projected(&path), old_projection);
        assert_eq!(measured(&path, &ids), old_metrics);
        assert_eq!(
            scalar(&path, "SELECT count(*) FROM injected_context_inputs"),
            0
        );
        assert_eq!(scalar(&path, content), 0);
    }

    // After the upgrade a new exact read's proof commits with its record; the
    // installed confirmations and their inputs stay exactly as they were.
    let mut store = Store::open(&path).unwrap();
    read_skilled(&mut store, true);
    drop(store);
    assert_eq!(confirmations(&path), old_rows);
    let upgraded = projected(&path);
    for row in &old_projection {
        assert!(upgraded.contains(row), "{row:?}");
    }
    assert!(upgraded.contains(&(sid(BODY), Some(0), Some(1), 1)));
    assert!(upgraded.contains(&(sid(ASK), Some(1), Some(1), 0)));
    assert_eq!(scalar(&path, content), 0);
    // A restart replays nothing and keeps every row.
    let mut store = Store::open(&path).unwrap();
    read_skilled(&mut store, false);
    drop(store);
    assert_eq!(projected(&path), upgraded);
    assert_eq!(confirmations(&path), old_rows);
}
