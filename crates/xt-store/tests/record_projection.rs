//! The shared record projection: one internal view joins each record's
//! session and stored input facts once and decides Human eligibility there;
//! `v_human_inputs` and `v_records` read it with their own unchanged columns
//! and row domains, and windowed reads touch each record and session once.
use rusqlite::{Connection, types::Value};
use serde_json::json;
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    human_input::{InputAdjustment, OriginManifest, SessionOrigin},
};

const USER: &str = "claude-user";
const JUDGE: &str = "claude-judge";
const CHILD: &str = "codex-child";
/// Human input adjustments bind only to Codex inputs.
const CODEX: &str = "codex-plain";

fn record(value: serde_json::Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}

fn text(uuid: &str, ts: Option<&str>, extra: serde_json::Value) -> CanonicalRecord {
    let mut value = json!({"uuid":uuid,"type":"user",
        "message":{"role":"user","content":[{"type":"text","text":"Synthetic typed text"}]}});
    if let Some(ts) = ts {
        value["timestamp"] = ts.into();
    }
    for (key, field) in extra.as_object().unwrap() {
        value[key] = field.clone();
    }
    record(value)
}

fn assistant(uuid: &str, model: &str, usage: Option<serde_json::Value>) -> CanonicalRecord {
    let mut message = json!({"id":format!("msg-{uuid}"),"role":"assistant","model":model,
        "content":[{"type":"text","text":"Synthetic answer"}]});
    if let Some(usage) = usage {
        message["usage"] = usage;
    }
    record(
        json!({"uuid":uuid,"type":"assistant","timestamp":"2026-10-06T10:05:00Z",
        "requestId":format!("req-{uuid}"),"message":message}),
    )
}

/// One record of every kind whose row domain, unknown or zero matters.
fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projection.sqlite");
    let mut store = Store::open(&path).unwrap();
    for (id, platform, source) in [
        (USER, "claude", SessionSource::Transcript),
        (JUDGE, "claude", SessionSource::Transcript),
        (CHILD, "codex", SessionSource::ReadersCli),
        (CODEX, "codex", SessionSource::ReadersCli),
    ] {
        let mut session = SessionMeta::new(id, platform, source);
        session.native_session_id = Some(id.trim_start_matches("codex-").into());
        store.upsert_session(&session, false).unwrap();
    }
    let at = Some("2026-10-06T10:00:00Z");
    store
        .upsert_records(
            USER,
            &[
                text("typed", at, json!({})),
                text("automated", at, json!({})),
                text("untimed", None, json!({})),
                text("meta", at, json!({"isMeta":true})),
                record(
                    json!({"uuid":"unknown","type":"user","timestamp":"2026-10-06T10:01:00Z",
                    "message":{"role":"user"}}),
                ),
                record(json!({"uuid":"unknown-automated","type":"user",
                    "timestamp":"2026-10-06T10:02:00Z","message":{"role":"user"}})),
                assistant(
                    "zero-usage",
                    "synthetic-model",
                    Some(json!({"input_tokens":0,"output_tokens":0,
                        "cache_read_input_tokens":0,"cache_creation_input_tokens":0})),
                ),
                assistant("no-usage", "synthetic-model", None),
                assistant("synthetic", "<synthetic>", None),
            ],
            false,
        )
        .unwrap();
    store
        .upsert_records(JUDGE, &[text("judged", at, json!({}))], false)
        .unwrap();
    store
        .upsert_records(CHILD, &[text("launched", at, json!({}))], false)
        .unwrap();
    store
        .upsert_records(
            CODEX,
            &[
                text("adjusted", at, json!({})),
                text("withheld", at, json!({})),
            ],
            false,
        )
        .unwrap();
    // A launched session's inputs are not a person's.
    store
        .import_human_session_origins(&OriginManifest {
            version: 1,
            sessions: vec![SessionOrigin {
                session_id: CHILD.into(),
                host: Host::Codex,
                native_session_id: "child".into(),
                parent_host: Host::Codex,
                parent_native_session_id: "parent".into(),
                method: "explicit_session_id".into(),
                evidence_id: "audit-1".into(),
                launch_id: "call-1".into(),
            }],
        })
        .unwrap();
    let adjustment = |uuid: &str, retained: Option<i64>, reason: &str| InputAdjustment {
        record_uuid: uuid.into(),
        session_id: CODEX.into(),
        original_ts: "2026-10-06T10:00:00Z".into(),
        original_length: 20,
        retained_length: retained,
        reason: reason.into(),
        native_item_id: None,
    };
    let report = store
        .apply_human_input_adjustments(&[
            adjustment("adjusted", Some(5), "question_reply"),
            adjustment("withheld", None, "heartbeat"),
        ])
        .unwrap();
    assert_eq!(report.applied, 2);
    drop(store);
    let sql = Connection::open(&path).unwrap();
    // A Claude Code task notification proof on a typed input and on one
    // ingestion could not classify; and a judge session.
    sql.execute_batch(&format!(
        "INSERT INTO task_notification_inputs VALUES
             ('automated','{USER}','claude_task_notification',1),
             ('unknown-automated','{USER}','claude_task_notification',1);
         UPDATE sessions SET kind='judge' WHERE session_id='{JUDGE}';"
    ))
    .unwrap();
    (directory, path)
}

fn rows(sql: &Connection, query: &str) -> Vec<Vec<Value>> {
    let mut statement = sql.prepare(query).unwrap();
    let width = statement.column_count();
    statement
        .query_map([], |row| (0..width).map(|i| row.get(i)).collect())
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn columns(sql: &Connection, view: &str) -> Vec<String> {
    sql.prepare(&format!(
        "SELECT name FROM pragma_table_info('{view}') ORDER BY cid"
    ))
    .unwrap()
    .query_map([], |row| row.get(0))
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}

fn uuids(sql: &Connection, view: &str) -> Vec<String> {
    sql.prepare(&format!("SELECT uuid FROM {view} ORDER BY uuid"))
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

const I: fn(i64) -> Value = Value::Integer;
const N: Value = Value::Null;

#[test]
fn public_views_keep_their_columns_and_row_domains() {
    let (_directory, path) = setup();
    let sql = Connection::open(&path).unwrap();
    assert_eq!(
        columns(&sql, "v_human_inputs"),
        [
            "uuid",
            "human_is_eligible",
            "human_text_len",
            "human_excluded"
        ]
    );
    assert_eq!(
        columns(&sql, "v_records"),
        [
            "uuid",
            "session_id",
            "type",
            "ts",
            "ts_ms",
            "api_message_id",
            "request_id",
            "is_sidechain",
            "role",
            "model",
            "is_tool_result_carrier",
            "text_len",
            "tool_use_count",
            "human_is_eligible",
            "human_text_len",
            "human_excluded",
            "is_human",
            "raw_is_human",
            "confirmed_automated_input",
            "is_command",
            "is_interrupted",
            "is_system_reminder",
            "parent_uuid",
            "agent_id",
            "subtype",
            "has_conflict",
            "host",
            "source_platform",
            "surface",
            "source",
            "kind",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_creation_tokens",
            "cache_creation_5m",
            "cache_creation_1h",
            "service_tier",
            "usage_observed",
        ]
    );
    // Human inputs cover every stored record; the record projection leaves out
    // metadata, judge sessions and the synthetic model; events need a time.
    let every = uuids(&sql, "records");
    assert_eq!(uuids(&sql, "v_human_inputs"), every);
    let kept: Vec<String> = every
        .iter()
        .filter(|uuid| !["meta", "judged", "synthetic"].contains(&uuid.as_str()))
        .cloned()
        .collect();
    assert_eq!(uuids(&sql, "v_records"), kept);
    let timed: Vec<String> = kept.into_iter().filter(|uuid| uuid != "untimed").collect();
    assert_eq!(uuids(&sql, "v_session_events"), timed);
}

#[test]
fn one_eligibility_decision_keeps_unknown_zero_and_the_legacy_flags_apart() {
    let (_directory, path) = setup();
    let sql = Connection::open(&path).unwrap();
    // uuid, raw, effective, confirmed, eligible, excluded, human length.
    let expected = [
        ("adjusted", I(1), I(1), I(0), I(1), I(0), I(5)),
        // A proof on a typed input: not human, and confirmed automated.
        ("automated", I(1), I(0), I(1), I(0), I(1), I(20)),
        // A launched session's input: not eligible, yet still effective human.
        ("launched", I(1), I(1), I(0), I(0), I(1), I(20)),
        ("typed", I(1), I(1), I(0), I(1), I(0), I(20)),
        // Unclassified stays unknown, even with a proof: eligibility is 0
        // while the effective value stays unknown and nothing is confirmed.
        ("unknown", N, N, I(0), N, I(0), N),
        ("unknown-automated", N, N, I(0), I(0), I(1), N),
        ("untimed", I(1), I(1), I(0), I(1), I(0), I(20)),
        ("withheld", I(1), I(1), I(0), I(0), I(1), N),
    ];
    let actual = rows(
        &sql,
        "SELECT uuid,raw_is_human,is_human,confirmed_automated_input,human_is_eligible,
                human_excluded,human_text_len
         FROM v_records WHERE role='user' ORDER BY uuid",
    );
    let expected: Vec<Vec<Value>> = expected
        .into_iter()
        .map(|(uuid, a, b, c, d, e, f)| vec![Value::Text(uuid.into()), a, b, c, d, e, f])
        .collect();
    assert_eq!(actual, expected);
    // Both public views read the same decision for every record they share,
    // and the human view decides it for the rows the record view leaves out.
    assert_eq!(
        rows(
            &sql,
            "SELECT count(*) FROM v_records r JOIN v_human_inputs h ON h.uuid=r.uuid
             WHERE h.human_is_eligible IS NOT r.human_is_eligible
                OR h.human_text_len IS NOT r.human_text_len
                OR h.human_excluded IS NOT r.human_excluded"
        ),
        [[I(0)]]
    );
    assert_eq!(
        rows(
            &sql,
            "SELECT uuid,human_is_eligible,human_text_len,human_excluded FROM v_human_inputs
             WHERE uuid IN ('judged','meta') ORDER BY uuid"
        ),
        [
            vec![Value::Text("judged".into()), I(1), I(20), I(0)],
            vec![Value::Text("meta".into()), I(0), I(20), I(0)],
        ]
    );
    // Measured zero usage is zero; no usage is unknown, not zero.
    assert_eq!(
        rows(
            &sql,
            "SELECT uuid,usage_observed,input_tokens,output_tokens FROM v_records
             WHERE type='assistant' ORDER BY uuid"
        ),
        [
            vec![Value::Text("no-usage".into()), I(0), N, N],
            vec![Value::Text("zero-usage".into()), I(1), I(0), I(0)],
        ]
    );
}

/// Each `SEARCH`/`SCAN` step of a plan, by the table alias it reads.
fn steps(sql: &Connection, query: &str) -> Vec<String> {
    sql.prepare(&format!("EXPLAIN QUERY PLAN {query}"))
        .unwrap()
        .query_map([0, 1], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .into_iter()
        .filter(|step| step.starts_with("SEARCH ") || step.starts_with("SCAN "))
        .collect()
}

#[test]
fn a_windowed_read_finds_each_record_and_session_once_through_the_time_index() {
    let (_directory, path) = setup();
    let sql = Connection::open(&path).unwrap();
    xt_store::timestamp::register_sqlite(&sql).unwrap();
    let reads = |steps: &[String], alias: &str| {
        steps
            .iter()
            .filter(|step| {
                step.strip_prefix("SEARCH ")
                    .or_else(|| step.strip_prefix("SCAN "))
                    .is_some_and(|rest| rest.split(' ').next() == Some(alias))
            })
            .count()
    };
    let events = steps(
        &sql,
        "SELECT session_id,uuid,ts,is_human,human_text_len,confirmed_automated_input,
                human_is_eligible FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2",
    );
    assert!(
        events.iter().any(|step| step.contains("records_ts")),
        "{events:?}"
    );
    assert_eq!(reads(&events, "r"), 1, "{events:?}");
    assert_eq!(reads(&events, "s"), 1, "{events:?}");
    // The selected-response read: the candidates through the time index,
    // their successors through the response index, each record once per side.
    let responses = steps(
        &sql,
        "SELECT session_id,ts,input_tokens FROM v_response_usage WHERE ts_ms>=?1 AND ts_ms<?2",
    );
    assert!(
        responses.iter().any(|step| step.contains("records_ts")),
        "{responses:?}"
    );
    assert!(
        responses
            .iter()
            .any(|step| step.contains("records_response")),
        "{responses:?}"
    );
    assert_eq!(reads(&responses, "r"), 2, "{responses:?}");
}
