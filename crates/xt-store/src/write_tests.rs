use crate::{CanonicalRecord, SessionMeta, SessionSource, Store, WriteStats};
use rusqlite::{
    limits::Limit,
    trace::{TraceEvent, TraceEventCodes},
};
use serde_json::json;
use std::cell::Cell;

thread_local! {
    static SELECTS: Cell<usize> = const { Cell::new(0) };
    static ROWS_READ: Cell<usize> = const { Cell::new(0) };
}

fn record(uuid: &str, number: usize) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":uuid,"type":"assistant","message":{
        "usage":{"input_tokens":number,"cache_read_input_tokens":0},
        "content":[{"type":"tool_use","name":"Read","input":{"number":number}}]
    }}))
    .unwrap()
}

fn session(store: &mut Store, id: &str) {
    store
        .upsert_session(
            &SessionMeta::new(id, "claude", SessionSource::Fixture),
            true,
        )
        .unwrap();
}

#[test]
fn large_replay_prefetches_only_requested_records_in_bounded_queries() {
    let mut store = Store::open_in_memory().unwrap();
    session(&mut store, "replay");
    session(&mut store, "unrelated");
    // Exercise a batch above the historical SQLite bind limit, even though the
    // bundled version permits more variables by default.
    store
        .connection
        .set_limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 999)
        .unwrap();
    let records = (0..10_000)
        .map(|n| record(&format!("record-{n:05}"), n))
        .collect::<Vec<_>>();
    store.upsert_records("replay", &records, true).unwrap();
    store
        .upsert_records("unrelated", &[record("outside-batch", 999)], true)
        .unwrap();
    let mut enrich = records.clone();
    for input in &mut enrich {
        input.message.model = Some("synthetic-model".into());
    }
    for (input, expected) in [
        (
            &enrich,
            WriteStats {
                enriched: 10_000,
                ..WriteStats::default()
            },
        ),
        (
            &records,
            WriteStats {
                ignored: 10_000,
                ..WriteStats::default()
            },
        ),
    ] {
        SELECTS.set(0);
        ROWS_READ.set(0);
        store.connection.trace_v2(
            TraceEventCodes::SQLITE_TRACE_STMT | TraceEventCodes::SQLITE_TRACE_ROW,
            Some(|event| {
                if let TraceEvent::Row(_) = event {
                    ROWS_READ.set(ROWS_READ.get() + 1);
                }
                if let TraceEvent::Stmt(statement, _) = event
                    && statement.sql().trim_start().starts_with("SELECT")
                {
                    SELECTS.set(SELECTS.get() + 1);
                }
            }),
        );
        let outcome = store.upsert_records("replay", input, true).unwrap();
        store.connection.trace_v2(TraceEventCodes::empty(), None);
        assert_eq!(outcome, expected);
        // 10k records + 10k usage rows + 10k tools + the target session.
        // Unrequested records/children must never be materialized by a full-table read.
        assert_eq!(ROWS_READ.get(), 30_001);
        let queries = SELECTS.get();
        assert!(
            (1..=62).contains(&queries),
            "10k replay must need at most 62 SELECTs, observed {queries}"
        );
    }
    let loaded = store.records("replay").unwrap();
    for (n, row) in loaded.iter().enumerate() {
        assert_eq!(row.uuid, format!("record-{n:05}"));
        assert_eq!(row.model.as_deref(), Some("synthetic-model"));
        assert_eq!(row.usage.as_ref().unwrap().input_tokens, Some(n as i64));
        assert_eq!(row.usage.as_ref().unwrap().cache_read_input_tokens, Some(0));
        assert_eq!(row.usage.as_ref().unwrap().output_tokens, None);
        assert_eq!(row.tool_uses[0].input_json, Some(json!({"number":n})));
    }
    assert_eq!(loaded.len(), 10_000);
}

#[test]
fn prefetch_keeps_sequential_duplicate_enrichment_conflicts_and_ownership() {
    let mut store = Store::open_in_memory().unwrap();
    session(&mut store, "target");
    session(&mut store, "owner");
    store
        .upsert_records("owner", &[record("foreign", 44)], true)
        .unwrap();
    let poor: CanonicalRecord =
        serde_json::from_value(json!({"uuid":"new", "type":"assistant", "message":{}})).unwrap();
    let rich = record("new", 10);
    let mut wrong_type = rich.clone();
    wrong_type.record_type = crate::model::RecordType::User;
    let mut later = rich.clone();
    later.message.usage.as_mut().unwrap().output_tokens = Some(7);
    let input = [
        poor,
        rich.clone(),
        wrong_type,
        later,
        rich,
        record("foreign", 55),
        record("foreign", 66),
    ];
    assert_eq!(
        store.upsert_records("target", &input, true).unwrap(),
        WriteStats {
            inserted: 1,
            enriched: 2,
            ignored: 4,
            dropped_no_uuid: 0
        }
    );
    let rows = store.records("target").unwrap();
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].has_conflict,
        "later enrichment must not clear a same-batch type conflict"
    );
    assert_eq!(rows[0].usage.as_ref().unwrap().input_tokens, Some(10));
    assert_eq!(rows[0].usage.as_ref().unwrap().output_tokens, Some(7));
    assert_eq!(rows[0].tool_uses.len(), 1);
    let foreign = store.records("owner").unwrap();
    assert_eq!(foreign.len(), 1);
    assert!(foreign[0].has_conflict);
    assert_eq!(foreign[0].usage.as_ref().unwrap().input_tokens, Some(44));
    assert_eq!(
        foreign[0].tool_uses[0].input_json,
        Some(json!({"number":44}))
    );
}
