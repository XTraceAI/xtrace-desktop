//! Codex and Cursor inputs the tool wrote itself — a subagent notification, an
//! interrupted-turn note, an app page record, a Cursor conversation summary
//! and the reader's own Cursor import banner — are automatic inputs, not a
//! person's messages, everywhere the index measures one: M-02's human
//! messages and the hours estimate. The reader's validated claim, made from a
//! structural marker in the native source, is the evidence; the same text
//! without a claim stays a person's. Inputs indexed before the claim was read
//! are corrected by reading them again, which every Codex and Cursor scan
//! does.
use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use xt_fixtures::TempDb;
use xt_metrics::{MetricsDb, TypingRate, Window};
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource,
    batch::IngestBatch,
    retention::RetentionMode,
    tool_sent::ToolSentKind::{self, *},
};

const CODEX: &str = "codex-019a0000-0000-7000-8000-000000000001";
const CURSOR: &str = "cursor-2a2a2a2a-1111-4222-8333-444444444444";

fn ms(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_millisecond()
}
fn at(hour: u32, minute: u32) -> String {
    format!("2026-09-30T{hour:02}:{minute:02}:00Z")
}
fn record(value: Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn input(uuid: &str, time: &str, text: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user","timestamp":time,
        "message":{"role":"user","content":[{"type":"text","text":text}]}}))
}
fn reply(uuid: &str, time: &str) -> CanonicalRecord {
    record(
        json!({"uuid":uuid,"type":"assistant","timestamp":time,"requestId":format!("req-{uuid}"),
        "message":{"id":format!("msg-{uuid}"),"role":"assistant","model":"fixture-model-v1",
            "content":[{"type":"text","text":"Synthetic reply"}],
            "usage":{"input_tokens":10,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
    )
}
fn tool_result(uuid: &str, time: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user","timestamp":time,
        "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"ok"}]}}))
}

const NOTE: &str = "<subagent_notification>\n{\"agent_path\":\"a\",\"status\":{\"completed\":\"done\"}}\n</subagent_notification>";
const ABORTED: &str =
    "<turn_aborted>\nThe user interrupted the previous turn on purpose.\n</turn_aborted>";
const PAGE: &str =
    "<external_codex_apps_open_page>{\"page_id\":null}</external_codex_apps_open_page>";

/// A Codex session: the person's request (9:00), a reply, a subagent
/// notification (9:05), an interrupted-turn note (9:06), an app page record
/// (9:07), a tool result, and a person's message that only pastes the same
/// tag (9:09), which the reader never claims.
fn codex_rows() -> Vec<(CanonicalRecord, Option<ToolSentKind>)> {
    vec![
        (input("p1", &at(9, 0), "fix the running indicator"), None),
        (reply("a1", &at(9, 1)), None),
        (
            input("n1", &at(9, 5), NOTE),
            Some(CodexSubagentNotification),
        ),
        (input("t1", &at(9, 6), ABORTED), Some(CodexTurnAborted)),
        (input("o1", &at(9, 7), PAGE), Some(CodexAppsOpenPage)),
        (tool_result("r1", &at(9, 8)), None),
        (input("q1", &at(9, 9), NOTE), None),
    ]
}

/// A Cursor session: the reader's banner (9:20), a person's ask, a reply and
/// Cursor's summary (9:30).
fn cursor_rows() -> Vec<(CanonicalRecord, Option<ToolSentKind>)> {
    vec![
        (
            input(
                "b1",
                &at(9, 20),
                "[Imported from Cursor · session 2a2a2a2a · cwd /synthetic]",
            ),
            Some(CursorImportBanner),
        ),
        (input("p2", &at(9, 21), "rename the module"), None),
        (reply("a2", &at(9, 22)), None),
        (
            input(
                "s1",
                &at(9, 30),
                "[Previous conversation summary]: the module was renamed",
            ),
            Some(CursorConversationSummary),
        ),
    ]
}

/// Read one session as the native reader's writer does: the claims travel
/// beside the records when `claims` is set, and not at all when it is not —
/// what a build before this change stored.
fn read(
    store: &mut xt_store::Store,
    session: &str,
    host: &str,
    rows: &[(CanonicalRecord, Option<ToolSentKind>)],
    claims: bool,
) {
    let mut meta = SessionMeta::new(session, host, SessionSource::ReadersCli);
    meta.native_session_id = Some(session.split_once('-').unwrap().1.into());
    let records = rows.iter().map(|row| row.0.clone()).collect::<Vec<_>>();
    let kinds = rows.iter().map(|row| row.1).collect::<Vec<_>>();
    let mut batch = IngestBatch::new(&meta, &records, false);
    if claims {
        batch.tool_sent = &kinds;
    }
    let saved = store.apply_ingest_batch(&batch).unwrap();
    assert!(
        saved
            .records
            .iter()
            .all(|row| row.disposition.is_accepted())
    );
}

fn database(claims: bool) -> TempDb {
    let mut db = TempDb::empty().unwrap();
    db.store_mut()
        .set_retention_mode(RetentionMode::MetadataOnly)
        .unwrap();
    read(db.store_mut(), CODEX, "codex", &codex_rows(), claims);
    read(db.store_mut(), CURSOR, "cursor", &cursor_rows(), claims);
    db
}

/// (uuid, raw is_human, effective is_human, human_is_eligible, confirmed) per
/// user input.
fn classified(db: &TempDb) -> Vec<(String, i64, i64, i64, i64)> {
    let connection = Connection::open(db.path()).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT uuid,raw_is_human,is_human,coalesce(human_is_eligible,-1),confirmed_automated_input
             FROM v_records WHERE type='user' ORDER BY ts",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn proofs(db: &TempDb) -> Vec<(String, String, String)> {
    let connection = Connection::open(db.path()).unwrap();
    let mut statement = connection
        .prepare("SELECT record_uuid,session_id,evidence_kind FROM tool_sent_inputs ORDER BY 1")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn window() -> Window {
    Window::new(ms("2026-09-30T00:00:00Z"), ms("2026-10-01T00:00:00Z")).unwrap()
}

fn human_messages(db: &TempDb) -> Option<u64> {
    MetricsDb::open(db.path())
        .unwrap()
        .counts(window(), TypingRate::default())
        .unwrap()
        .human_messages
}

#[test]
fn a_claimed_input_is_automatic_and_nothing_else_is() {
    let db = database(true);
    assert_eq!(
        classified(&db),
        [
            ("p1".into(), 1, 1, 1, 0),
            // Ingestion's own classification is untouched; the proof is read
            // as an override.
            ("n1".into(), 1, 0, 0, 1),
            ("t1".into(), 1, 0, 0, 1),
            ("o1".into(), 1, 0, 0, 1),
            ("r1".into(), 0, 0, 0, 0),
            // The same text without a claim is a person's message: the text
            // is never the evidence.
            ("q1".into(), 1, 1, 1, 0),
            ("b1".into(), 1, 0, 0, 1),
            ("p2".into(), 1, 1, 1, 0),
            ("s1".into(), 1, 0, 0, 1),
        ]
    );
    assert_eq!(
        proofs(&db),
        [
            ("b1".into(), CURSOR.into(), "cursor_import_banner".into()),
            (
                "n1".into(),
                CODEX.into(),
                "codex_subagent_notification".into()
            ),
            ("o1".into(), CODEX.into(), "codex_apps_open_page".into()),
            (
                "s1".into(),
                CURSOR.into(),
                "cursor_conversation_summary".into()
            ),
            ("t1".into(), CODEX.into(), "codex_turn_aborted".into()),
        ]
    );
    // p1, q1 and p2 stay the person's; five tool-sent inputs no longer count.
    assert_eq!(human_messages(&database(false)), Some(8));
    assert_eq!(human_messages(&db), Some(3));
}

#[test]
fn an_index_built_before_the_claims_is_corrected_by_reading_it_again() {
    // An older build stored every tool-sent input as the person's message,
    // with a person preview of each.
    let mut db = database(false);
    assert_eq!(proofs(&db), []);
    let previews = |db: &TempDb| -> Vec<(String, String)> {
        let connection = Connection::open(db.path()).unwrap();
        let mut statement = connection
            .prepare("SELECT record_uuid,kind FROM record_previews ORDER BY 1")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    assert!(previews(&db).contains(&("n1".into(), "person".into())));
    let records = |db: &TempDb| {
        let connection = Connection::open(db.path()).unwrap();
        let mut statement = connection
            .prepare("SELECT uuid,is_human,has_conflict,text_len,first_seen_at FROM records ORDER BY uuid")
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let stored = records(&db);
    // The next scan reads the same sessions again, now with the claims.
    // Twice, to show it settles.
    for _ in 0..2 {
        read(db.store_mut(), CODEX, "codex", &codex_rows(), true);
        read(db.store_mut(), CURSOR, "cursor", &cursor_rows(), true);
        assert_eq!(classified(&db), classified(&database(true)));
        assert_eq!(proofs(&db), proofs(&database(true)));
        // Not one stored record changed or became conflicted.
        assert_eq!(records(&db), stored);
    }
    assert_eq!(human_messages(&db), Some(3));
    // Each corrected input's person preview is withdrawn; the people's stay.
    assert_eq!(
        previews(&db),
        [
            ("p1".into(), "person".into()),
            ("p2".into(), "person".into()),
            ("q1".into(), "person".into()),
        ]
    );
}

#[test]
fn a_claim_binds_only_to_a_human_input_of_its_own_host() {
    let mut db = TempDb::empty().unwrap();
    // A Cursor kind on a Codex input, a Codex kind on a Cursor input, and a
    // claim on a tool result (not a human input) all abstain.
    let codex = vec![
        (
            input("x1", &at(9, 0), NOTE),
            Some(CursorConversationSummary),
        ),
        (
            tool_result("x2", &at(9, 1)),
            Some(CodexSubagentNotification),
        ),
        (reply("x3", &at(9, 2)), Some(CodexTurnAborted)),
    ];
    let cursor = vec![(
        input("y1", &at(9, 3), NOTE),
        Some(CodexSubagentNotification),
    )];
    read(db.store_mut(), CODEX, "codex", &codex, true);
    read(db.store_mut(), CURSOR, "cursor", &cursor, true);
    assert_eq!(proofs(&db), []);
    assert_eq!(human_messages(&db), Some(2));
    // A Claude session never takes one either.
    let claude = vec![(
        input("z1", &at(9, 4), NOTE),
        Some(CodexSubagentNotification),
    )];
    read(db.store_mut(), "claude-synthetic", "claude", &claude, true);
    assert_eq!(proofs(&db), []);
}

#[test]
fn the_hours_estimate_counts_only_the_persons_characters() {
    let hours = |db: &TempDb| {
        MetricsDb::open(db.path())
            .unwrap()
            .human_time(window(), TypingRate::new(60).unwrap(), TimeZone::UTC)
            .unwrap()
            .human_minutes_est
            .unwrap()
    };
    let before = hours(&database(false));
    let after = hours(&database(true));
    let person = ["fix the running indicator", NOTE, "rename the module"]
        .iter()
        .map(|text| text.chars().count())
        .sum::<usize>() as f64;
    assert!(after < before, "{after} < {before}");
    assert!((after - person / 60.0).abs() < 1e-9, "{after} vs {person}");
}
