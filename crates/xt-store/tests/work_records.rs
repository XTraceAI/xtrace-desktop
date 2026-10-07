//! The Sessions list reads two per-session answers straight from the base
//! tables: a session's own first event and its model uses. Both are built from
//! the same rule text as the shared views (`xt_store::views`), and these tests
//! hold them to the views' own answers, so a session's start and model never
//! differ from sessions per day (M-16, `v_session_events`) or from the
//! responses the cost report and the Effort card select (`v_response_usage`).
use serde_json::json;
use std::collections::BTreeMap;
use xt_store::session_model::{self, ModelUses};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, Store};

fn record(
    uuid: &str,
    ts: Option<&str>,
    model: Option<&str>,
    output: Option<u64>,
) -> CanonicalRecord {
    let mut message = json!({"role":"assistant","content":[]});
    if let Some(model) = model {
        message["model"] = json!(model);
    }
    if let Some(output) = output {
        message["usage"] = json!({"input_tokens":1,"output_tokens":output,
            "cache_read_input_tokens":0,"cache_creation_input_tokens":0});
    }
    let mut value = json!({"uuid":uuid,"type":"assistant","message":message});
    if let Some(ts) = ts {
        value["timestamp"] = json!(ts);
    }
    serde_json::from_value(value).unwrap()
}
fn keyed(mut record: CanonicalRecord, message: &str, request: &str) -> CanonicalRecord {
    record.api_message_id = Some(message.into());
    record.request_id = Some(request.into());
    record
}
fn meta(mut record: CanonicalRecord) -> CanonicalRecord {
    let mut value = serde_json::to_value(&record).unwrap();
    value["isMeta"] = json!(true);
    record = serde_json::from_value(value).unwrap();
    record
}

fn build() -> (tempfile::TempDir, rusqlite::Connection) {
    let temp = tempfile::TempDir::new().unwrap();
    let path = temp.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    let t = |minute: u32| format!("2026-09-02T10:{minute:02}:00Z");
    let sessions: Vec<(&str, &str, Vec<CanonicalRecord>)> = vec![
        (
            "a",
            "claude",
            vec![
                // Earlier than any work event: neither is one.
                meta(record("a-meta", Some(&t(0)), Some("opus"), None)),
                record("a-synthetic", Some(&t(1)), Some("<synthetic>"), Some(1)),
                record("a-untimed", None, Some("opus"), Some(1)),
                // One response, three snapshots: the latest counts once.
                keyed(
                    record("a-r1-1", Some(&t(5)), Some("opus"), Some(1)),
                    "m1",
                    "q1",
                ),
                keyed(
                    record("a-r1-2", Some(&t(6)), Some("opus"), Some(7)),
                    "m1",
                    "q1",
                ),
                keyed(
                    record("a-r1-3", Some(&t(6)), Some("haiku"), Some(9)),
                    "m1",
                    "q1",
                ),
                // Equal instants spelled differently: the larger UUID wins.
                keyed(
                    record(
                        "a-r2-z",
                        Some("2026-09-02T10:07:00Z"),
                        Some("sonnet"),
                        Some(2),
                    ),
                    "m2",
                    "q2",
                ),
                keyed(
                    record(
                        "a-r2-a",
                        Some("2026-09-02T05:07:00-05:00"),
                        Some("opus"),
                        Some(3),
                    ),
                    "m2",
                    "q2",
                ),
                // Its latest snapshot is stored under session b.
                keyed(
                    record("a-r3", Some(&t(8)), Some("opus"), Some(4)),
                    "m3",
                    "q3",
                ),
                // Whitespace-only IDs are not keyed: each is its own response.
                keyed(
                    record("a-blank-1", Some(&t(9)), Some("opus"), Some(1)),
                    " ",
                    "q4",
                ),
                keyed(
                    record("a-blank-2", Some(&t(9)), Some("opus"), Some(1)),
                    " ",
                    "q4",
                ),
                // A blank model is no model.
                record("a-nameless", Some(&t(10)), Some("  "), Some(50)),
            ],
        ),
        (
            "b",
            "claude",
            vec![
                keyed(
                    record("b-r3", Some(&t(20)), Some("haiku"), Some(5)),
                    "m3",
                    "q3",
                ),
                record("b-cursorish", Some(&t(21)), Some("no-usage"), None),
            ],
        ),
        (
            "c",
            "cursor",
            vec![
                record("c-1", Some(&t(30)), Some("grok"), None),
                record("c-2", Some(&t(31)), Some("grok"), None),
                record("c-3", Some(&t(32)), Some("  "), None),
            ],
        ),
    ];
    for (id, host, records) in &sessions {
        store
            .upsert_session(
                &SessionMeta::new(*id, *host, SessionSource::Transcript),
                false,
            )
            .unwrap();
        store.upsert_records(id, records, false).unwrap();
    }
    let connection = rusqlite::Connection::open(&path).unwrap();
    xt_store::timestamp::register_sqlite(&connection).unwrap();
    (temp, connection)
}

#[test]
fn a_sessions_shown_start_is_its_first_event_in_v_session_events() {
    let (_temp, connection) = build();
    for id in ["a", "b", "c"] {
        let expected: Option<i64> = connection
            .query_row(
                "SELECT min(ts_ms) FROM v_session_events WHERE session_id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        let row = xt_store::session_list::exact(&connection, id)
            .unwrap()
            .unwrap();
        if row.host == "claude" {
            assert_eq!(row.started_at_ms, expected, "{id}");
        }
    }
    // The meta and `<synthetic>` records before it set no start.
    let a = xt_store::session_list::exact(&connection, "a")
        .unwrap()
        .unwrap();
    assert_eq!(
        a.started_at_ms,
        Some(
            xt_store::timestamp::parse("2026-09-02T10:05:00Z")
                .unwrap()
                .1
        )
    );
}

#[test]
fn model_uses_are_the_selected_responses_of_v_response_usage() {
    let (_temp, connection) = build();
    let mut reference = BTreeMap::<String, ModelUses>::new();
    let mut responses = connection
        .prepare(
            "SELECT session_id,model,output_tokens FROM v_response_usage WHERE model IS NOT NULL",
        )
        .unwrap();
    let mut rows = responses.query([]).unwrap();
    while let Some(row) = rows.next().unwrap() {
        let model: String = row.get(1).unwrap();
        let tokens: Option<i64> = row.get(2).unwrap();
        reference
            .entry(row.get(0).unwrap())
            .or_default()
            .add_responses(&model, 1, tokens.unwrap_or(0) as u64);
    }
    drop(rows);
    let mut unmetered = connection
        .prepare(
            "SELECT session_id,model FROM v_records
             WHERE type='assistant' AND usage_observed=0 AND model IS NOT NULL",
        )
        .unwrap();
    let mut rows = unmetered.query([]).unwrap();
    while let Some(row) = rows.next().unwrap() {
        let model: String = row.get(1).unwrap();
        reference
            .entry(row.get(0).unwrap())
            .or_default()
            .add_unmetered_records(&model, 1);
    }
    drop(rows);
    let read = session_model::whole_sessions(&connection, &["a", "b", "c"]).unwrap();
    let shown = |uses: &BTreeMap<String, ModelUses>| {
        uses.iter()
            .filter_map(|(id, uses)| {
                uses.most_used()
                    .map(|most| (id.clone(), format!("{uses:?}"), most))
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(shown(&read), shown(&reference));
    let most = |id: &str| read[id].most_used().unwrap();
    // a: haiku (r1's latest), sonnet (r2's larger UUID at an equal instant),
    // opus three times (two unkeyed blanks and the untimed response); r3
    // counts under b, where its latest snapshot is.
    assert_eq!(
        (most("a").model.as_str(), most("a").other_models),
        ("opus", 2)
    );
    assert_eq!(
        (most("b").model.as_str(), most("b").other_models),
        ("haiku", 1)
    );
    assert_eq!(
        (most("c").model.as_str(), most("c").other_models),
        ("grok", 0)
    );
}
