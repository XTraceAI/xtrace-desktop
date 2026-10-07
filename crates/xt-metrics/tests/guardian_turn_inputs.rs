//! A Guardian turn confirmation is the historical correction for one saved
//! Codex reviewer input. It inherits the confirmed-input semantics exactly:
//! that one input stops counting as human, its session's other inputs do not,
//! and no agent work, token or all-event measurement moves. A schema 11 index
//! upgrades to it without touching its tool-call confirmations or any metric.

use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use xt_fixtures::TempDb;
use xt_metrics::{MetricsDb, TypingRate, Window};
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    confirmation::{
        AutomatedInputProof, ConfirmationDisposition, EvidenceKind, GuardianEvidenceKind,
        GuardianTurnProof,
    },
};

// Synthetic canonical identities; none names a real thread or turn.
const NATIVE: &str = "abcdef00-0000-7000-8000-00000000aaaa";
const GUARDIAN: &str = "codex-abcdef00-0000-7000-8000-00000000aaaa";
const PARENT: &str = "abcdef00-0000-7000-8000-00000000dddd";
const DISPATCHED: &str = "synthetic-dispatched";

/// The `v_records` definition an installed schema 11 index holds.
const SCHEMA_11_RECORDS_VIEW: &str = "CREATE VIEW v_records AS
SELECT r.uuid,r.session_id,r.type,r.ts,r.ts_ms,r.api_message_id,r.request_id,
       r.is_sidechain,r.role,r.model,r.is_tool_result_carrier,r.text_len,
       r.tool_use_count,
       -- Effective M-02 eligibility: a structurally confirmed automated input
       -- is not a human message. raw_is_human keeps ingestion's classification.
       CASE WHEN a.record_uuid IS NULL THEN r.is_human ELSE 0 END AS is_human,
       r.is_human AS raw_is_human,a.record_uuid IS NOT NULL AS confirmed_automated_input,
       r.is_command,r.is_interrupted,r.is_system_reminder,
       r.parent_uuid,r.agent_id,r.subtype,r.has_conflict,
       s.host,s.source_platform,s.surface,s.source,s.kind,
       u.input_tokens,u.output_tokens,u.cache_read_tokens,u.cache_creation_tokens,
       u.cache_creation_5m,u.cache_creation_1h,u.service_tier,u.uuid IS NOT NULL AS usage_observed
FROM records r JOIN sessions s ON s.session_id=r.session_id
LEFT JOIN usage u ON u.uuid=r.uuid
LEFT JOIN confirmed_automated_inputs a
    ON a.record_uuid=r.uuid AND a.session_id=r.session_id AND r.is_human=1
WHERE r.is_meta=0 AND s.kind='user' AND (r.model IS NULL OR r.model<>'<synthetic>');";

fn ms(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn at(hour: u32, minute: u32) -> String {
    format!("2026-09-07T{hour:02}:{minute:02}:00Z")
}
/// A synthetic canonical record UUID ending in `n`.
fn id(n: u32) -> String {
    format!("abcdef00-0000-5000-8000-{n:012x}")
}
fn turn(n: u32) -> String {
    format!("abcdef00-0000-7000-9000-{n:012x}")
}
fn record(value: Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn input(uuid: &str, hour: u32, minute: u32, text: &str) -> CanonicalRecord {
    record(
        json!({"uuid":uuid,"type":"user","timestamp":at(hour, minute),
        "message":{"role":"user","content":[{"type":"text","text":text}]}}),
    )
}
fn tool(uuid: &str, hour: u32, minute: u32) -> CanonicalRecord {
    record(
        json!({"uuid":uuid,"type":"assistant","timestamp":at(hour, minute),
        "message":{"id":format!("msg-{uuid}"),"role":"assistant","model":"synthetic-model",
            "content":[{"type":"tool_use","name":"Read","input":{"path":format!("synthetic/{uuid}")}}],
            "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":1,"cache_creation_input_tokens":2}}}),
    )
}
fn result(uuid: &str, hour: u32, minute: u32) -> CanonicalRecord {
    record(
        json!({"uuid":uuid,"type":"user","timestamp":at(hour, minute),
        "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"synthetic"}]}}),
    )
}
fn reply(uuid: &str, hour: u32, minute: u32) -> CanonicalRecord {
    record(
        json!({"uuid":uuid,"type":"assistant","timestamp":at(hour, minute),
        "message":{"id":format!("msg-{uuid}"),"role":"assistant","model":"synthetic-model",
            "content":[{"type":"text","text":"Synthetic reply"}],
            "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":1,"cache_creation_input_tokens":2}}}),
    )
}

/// The Guardian reviewer's Codex session: a first reviewer input whose turn is
/// proven, work, a second reviewer input nothing proves, more work, and a
/// later input a person typed that ends the second input's stretch. The first
/// input's hour holds no other interval, so its estimate is its own.
fn guardian_rows() -> Vec<CanonicalRecord> {
    vec![
        input(&id(1), 9, 0, &"x".repeat(400)),
        tool(&id(2), 9, 1),
        result(&id(3), 9, 2),
        reply(&id(4), 9, 3),
        input(&id(10), 10, 0, "Synthetic human"),
        tool(&id(11), 10, 1),
        result(&id(12), 10, 2),
        input(&id(13), 10, 3, "Synthetic second reviewer request"),
        tool(&id(14), 10, 4),
        result(&id(15), 10, 5),
        input(&id(16), 10, 10, "Synthetic human again"),
        reply(&id(17), 10, 11),
    ]
}

/// A Claude session with eight inputs a parent agent's tool calls dispatched,
/// already confirmed on the installed index, and one a person typed.
fn dispatched_rows() -> Vec<CanonicalRecord> {
    let mut rows = vec![input("human", 12, 0, "Synthetic human")];
    for n in 0..8 {
        rows.push(input(
            &format!("dispatched-{n}"),
            12,
            2 * n + 1,
            "Synthetic dispatched",
        ));
        rows.push(reply(&format!("reply-{n}"), 12, 2 * n + 2));
    }
    rows
}

fn build() -> TempDb {
    let mut db = TempDb::empty().unwrap();
    let store = db.store_mut();
    let mut meta = SessionMeta::new(GUARDIAN, "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some(NATIVE.into());
    meta.surface = Some("cli".into());
    store.upsert_session(&meta, false).unwrap();
    store
        .upsert_records(GUARDIAN, &guardian_rows(), false)
        .unwrap();
    let mut meta = SessionMeta::new(DISPATCHED, "claude", SessionSource::Transcript);
    meta.native_session_id = Some(DISPATCHED.into());
    meta.surface = Some("cli".into());
    store.upsert_session(&meta, false).unwrap();
    store
        .upsert_records(DISPATCHED, &dispatched_rows(), false)
        .unwrap();
    let proofs: Vec<_> = (0..8)
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
        .collect();
    let report = store
        .apply_automated_input_confirmations(&proofs, 1_000)
        .unwrap();
    assert_eq!(report.dispositions, [ConfirmationDisposition::Confirmed; 8]);
    db
}

/// Leave exactly what a schema 11 build wrote: no migration 12 or later, no
/// Guardian or injected context table, and that build's record projection.
fn rewind_to_schema_11(path: &std::path::Path) {
    let sql = Connection::open(path).unwrap();
    sql.execute_batch(&format!(
        "ALTER TABLE pull_requests DROP COLUMN manual_failed_at; DELETE FROM schema_version WHERE version>=12;
         DROP VIEW v_response_usage; DROP VIEW v_usage_records;
         DROP VIEW v_session_events; DROP VIEW v_records;
         DROP TABLE guardian_turn_inputs;
         DROP TABLE injected_context_inputs; DROP TABLE IF EXISTS tool_sent_inputs; DROP TABLE IF EXISTS task_notification_inputs; DROP TABLE IF EXISTS record_previews; DROP TABLE IF EXISTS session_child_checks; DROP TABLE IF EXISTS session_child_facts; DROP TABLE IF EXISTS human_input_adjustments; DROP TABLE IF EXISTS human_session_origins; DROP TABLE claude_launch_groups; DROP TABLE claude_launch_group_members; DROP TABLE claude_launch_candidates; DROP TABLE claude_launch_staged_candidates;
         {SCHEMA_11_RECORDS_VIEW}
         {}; {}; {};",
        xt_store::views::USAGE_RECORDS,
        xt_store::views::RESPONSE_USAGE,
        xt_store::views::SESSION_EVENTS,
    ))
    .unwrap();
}

fn guardian_proof() -> GuardianTurnProof {
    GuardianTurnProof {
        record_uuid: id(1),
        session_id: GUARDIAN.into(),
        native_session_id: NATIVE.into(),
        turn_id: turn(1),
        parent_native_session_id: PARENT.into(),
        parent_turn_id: format!("abcdef00-0000-7000-a000-{:012x}", 1),
        evidence_kind: GuardianEvidenceKind::GuardianTurnDispatch,
        matcher_version: 1,
    }
}

/// The tool-call confirmations exactly as stored: rowid and every column's
/// type and value.
fn dispatch_rows(path: &std::path::Path) -> Vec<String> {
    Connection::open(path)
        .unwrap()
        .prepare(
            "SELECT rowid||'|'||quote(record_uuid)||'|'||quote(session_id)||'|'||
                 quote(native_session_id)||'|'||quote(evidence_kind)||'|'||quote(matcher_version)||'|'||
                 quote(parent_host)||'|'||quote(parent_session_id)||'|'||quote(parent_tool_call_id)||'|'||
                 quote(parent_operation_index)||'|'||quote(parent_result_id)||'|'||quote(confirmed_at)
             FROM confirmed_automated_inputs ORDER BY rowid",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
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

/// Every dashboard and session measurement this change could reach.
fn measured(path: &std::path::Path) -> Value {
    let m = MetricsDb::open(path).unwrap();
    let rate = TypingRate::default();
    let ids = [GUARDIAN, DISPATCHED];
    json!({
        "counts": m.counts(window(), rate).unwrap(),
        "human": m.human_time(window(), rate, TimeZone::UTC).unwrap(),
        "tokens": m.tokens(window(), TimeZone::UTC).unwrap(),
        "spans": m.active_spans(window()).unwrap(),
        "concurrency": m.concurrency(window()).unwrap(),
        "hands_off": m.hands_off(window()).unwrap(),
        "session_hands_off": m.session_hands_off(window(), &ids).unwrap(),
        "windows": m.session_windows(window(), &ids).unwrap(),
        "favorite": m.favorite_model(window()).unwrap(),
        "stretches": ids.map(|id| m.session_stretches(window(), id).unwrap()),
    })
}

fn view_sql(path: &std::path::Path, name: &str) -> String {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='view' AND name=?1",
            [name],
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
fn a_schema_11_index_upgrades_with_its_eight_confirmations_and_metrics_unchanged() {
    let db = build();
    let path = db.path().to_owned();
    let old_metrics = measured(&path);
    rewind_to_schema_11(&path);
    assert!(MetricsDb::open(&path).is_err());
    assert!(!table_exists(&path, "guardian_turn_inputs"));
    assert_eq!(
        view_sql(&path, "v_records"),
        SCHEMA_11_RECORDS_VIEW.trim_end_matches(';')
    );
    let old_rows = dispatch_rows(&path);
    assert_eq!(old_rows.len(), 8);
    let old_projection = projected(&path);

    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 23);
        assert!(
            store
                .guardian_turn_confirmations(GUARDIAN)
                .unwrap()
                .is_empty()
        );
        drop(store);
        assert!(
            view_sql(&path, "v_record_metadata").contains("guardian_turn_inputs"),
            "view rebuilt on open"
        );
        assert_eq!(dispatch_rows(&path), old_rows);
        assert_eq!(projected(&path), old_projection);
        assert_eq!(measured(&path), old_metrics);
    }

    // After the upgrade one proof corrects its one input; the eight stay.
    let mut store = Store::open(&path).unwrap();
    let report = store
        .apply_guardian_turn_confirmations(&[guardian_proof()], 2_000)
        .unwrap();
    assert_eq!(report.dispositions, [ConfirmationDisposition::Confirmed]);
    drop(store);
    let corrected = projected(&path);
    for (before, after) in old_projection.iter().zip(&corrected) {
        if before.0 == id(1) {
            assert_eq!((before.1, before.3), (Some(1), 0));
            assert_eq!(after, &(id(1), Some(0), Some(1), 1));
        } else {
            assert_eq!(before, after);
        }
    }
    assert_eq!(dispatch_rows(&path), old_rows);
    // A restart replays nothing and keeps the correction.
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .apply_guardian_turn_confirmations(&[guardian_proof()], 3_000)
            .unwrap()
            .dispositions,
        [ConfirmationDisposition::AlreadyConfirmed]
    );
    assert_eq!(projected(&path), corrected);
    assert_eq!(dispatch_rows(&path), old_rows);
}

#[test]
fn one_guardian_proof_moves_only_its_input_s_human_measurements() {
    let unconfirmed = build();
    let mut confirmed = build();
    let report = confirmed
        .store_mut()
        .apply_guardian_turn_confirmations(&[guardian_proof()], 2_000)
        .unwrap();
    assert_eq!(report.dispositions, [ConfirmationDisposition::Confirmed]);
    assert_eq!(report.affected_sessions.len(), 1);
    assert_eq!(report.affected_sessions[0].session_id, GUARDIAN);
    let (before, after) = (measured(unconfirmed.path()), measured(confirmed.path()));

    // M-02: one fewer human message and exactly its 400 characters fewer.
    // The eight dispatched inputs were already neutral on both sides.
    let chars = |m: &Value| m["counts"]["human_chars"].as_u64().unwrap();
    assert_eq!(before["counts"]["human_messages"], 5);
    assert_eq!(after["counts"]["human_messages"], 4);
    assert_eq!(chars(&before) - chars(&after), 400);
    // M-03: the proven input no longer opens the turn its work answered.
    assert_eq!(before["counts"]["assistant_turns"], 5);
    assert_eq!(after["counts"]["assistant_turns"], 4);
    // M-07: its hour held no other human interval, so exactly its typing
    // estimate (400 characters at 200 per minute) leaves the total.
    let minutes = |m: &Value| m["human"]["human_minutes_est"].as_f64().unwrap();
    assert_eq!(minutes(&before) - minutes(&after), 2.0);
    // Per session: only the Guardian session loses a human message.
    let windows = |m: &Value, id: &str| m["windows"][id].clone();
    assert_eq!(windows(&before, GUARDIAN)["state"], "indexed");
    assert_eq!(windows(&before, GUARDIAN)["human_messages"], 4);
    assert_eq!(windows(&after, GUARDIAN)["human_messages"], 3);
    assert_eq!(windows(&before, DISPATCHED), windows(&after, DISPATCHED));
    for field in ["events", "tool_calls", "tokens", "agent_ms"] {
        assert_eq!(
            windows(&before, GUARDIAN)[field],
            windows(&after, GUARDIAN)[field],
            "{field}"
        );
    }
    // M-09: the unproven second reviewer input still ends the person's
    // stretch; the proven input seeds none of its own.
    let starts = |m: &Value| {
        m["stretches"][0]["stretches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["start_uuid"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(starts(&before), [id(1), id(10), id(13)]);
    assert_eq!(starts(&after), [id(10), id(13)]);
    assert_eq!(
        (
            before["hands_off"]["n"].clone(),
            after["hands_off"]["n"].clone()
        ),
        (json!(3), json!(2))
    );
    assert_eq!(after["stretches"][1], before["stretches"][1]);
    // M-04/M-05/M-06, assistant records, tool calls and sessions are identical.
    for field in ["tokens", "spans", "concurrency"] {
        assert_eq!(before[field], after[field], "{field}");
    }
    for field in ["sessions", "assistant_records", "tool_calls"] {
        assert_eq!(before["counts"][field], after["counts"][field], "{field}");
    }
    assert_eq!(
        before["human"]["agent_minutes"],
        after["human"]["agent_minutes"]
    );
}

#[test]
fn a_guardian_input_removes_its_characters_regardless_of_other_input_timing() {
    // Agent work at 0, a proven reviewer input at 5, a person at 10: the
    // person's interval from the agent event already covers the input's, so
    // only the message count moves.
    let rows = vec![
        reply(&id(1), 9, 0),
        input(&id(2), 9, 5, "Synthetic reviewer request"),
        input(&id(3), 9, 10, "Synthetic human"),
        reply(&id(4), 9, 11),
    ];
    let seeded = || {
        let mut db = TempDb::empty().unwrap();
        let mut meta = SessionMeta::new(GUARDIAN, "codex", SessionSource::ReadersCli);
        meta.native_session_id = Some(NATIVE.into());
        meta.surface = Some("cli".into());
        db.store_mut().upsert_session(&meta, false).unwrap();
        db.store_mut()
            .upsert_records(GUARDIAN, &rows, false)
            .unwrap();
        db
    };
    let unconfirmed = seeded();
    let mut confirmed = seeded();
    let proof = GuardianTurnProof {
        record_uuid: id(2),
        ..guardian_proof()
    };
    assert_eq!(
        confirmed
            .store_mut()
            .apply_guardian_turn_confirmations(&[proof], 2_000)
            .unwrap()
            .dispositions,
        [ConfirmationDisposition::Confirmed]
    );
    let rate = TypingRate::default();
    let (before, after) = (
        MetricsDb::open(unconfirmed.path()).unwrap(),
        MetricsDb::open(confirmed.path()).unwrap(),
    );
    let human = |m: &MetricsDb| {
        m.human_time(window(), rate, TimeZone::UTC)
            .unwrap()
            .human_minutes_est
    };
    assert_eq!(
        human(&before),
        Some(("Synthetic reviewer request".len() + "Synthetic human".len()) as f64 / 200.0)
    );
    assert_eq!(human(&after), Some("Synthetic human".len() as f64 / 200.0));
    assert_eq!(
        (
            before.counts(window(), rate).unwrap().human_messages,
            after.counts(window(), rate).unwrap().human_messages
        ),
        (Some(2), Some(1))
    );
    assert_eq!(
        before.active_spans(window()).unwrap(),
        after.active_spans(window()).unwrap()
    );
}
