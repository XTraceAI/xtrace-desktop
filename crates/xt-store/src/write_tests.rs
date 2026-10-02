use crate::{
    CanonicalRecord, SessionMeta, SessionSource, Store, WriteStats,
    batch::{IngestBatch, SourceCursor},
};
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
        .set_retention_mode(crate::retention::RetentionMode::FullContent)
        .unwrap();
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
    for (input, expected, composed) in [
        (
            &enrich,
            WriteStats {
                enriched: 10_000,
                ..WriteStats::default()
            },
            false,
        ),
        (
            &records,
            WriteStats {
                ignored: 10_000,
                ..WriteStats::default()
            },
            false,
        ),
        (
            &records,
            WriteStats {
                ignored: 10_000,
                ..WriteStats::default()
            },
            true,
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
        let outcome = if composed {
            let metadata = SessionMeta::new("replay", "claude", SessionSource::Fixture);
            let cursor = SourceCursor {
                source: SessionSource::Fixture,
                cursor_key: "large-replay".into(),
                position: 10_000,
                updated_at: 100,
            };
            let mut batch = IngestBatch::new(&metadata, input, true);
            batch.cursor = Some(&cursor);
            store.apply_ingest_batch(&batch).unwrap().stats
        } else {
            store.upsert_records("replay", input, true).unwrap()
        };
        store.connection.trace_v2(TraceEventCodes::empty(), None);
        assert_eq!(outcome, expected);
        // 10k records + 10k usage rows + 10k tools + the target session.
        // Unrequested records/children must never be materialized by a full-table read.
        // The composed boundary additionally reads session metadata once before
        // the shared record writer. Every transaction reads the retention setting
        // once (one explicit full-content policy row); advancing the cursor adds no SELECT.
        assert_eq!(ROWS_READ.get(), 30_002 + usize::from(composed));
        let queries = SELECTS.get();
        // Shared-context detection adds one constant, empty-result lookup.
        let maximum = 64 + usize::from(composed);
        assert!(
            (1..=maximum).contains(&queries),
            "10k replay must need at most {maximum} SELECTs, observed {queries}"
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

#[test]
fn malformed_metadata_on_conflicting_uuids_rolls_back_the_whole_batch() {
    for conflict in ["other-session", "other-type", "same-batch-type"] {
        for invalid in ["platform", "evidence-source", "evidence-version"] {
            for keep_content in [false, true] {
                let mut store = Store::open_in_memory().unwrap();
                session(&mut store, "target");
                session(&mut store, "owner");
                if conflict != "same-batch-type" {
                    let owner = if conflict == "other-session" {
                        "owner"
                    } else {
                        "target"
                    };
                    store
                        .upsert_records(owner, &[record("existing", 1)], true)
                        .unwrap();
                }
                let before_counts = store.counts().unwrap();
                let before_sessions = [
                    store.session("target").unwrap(),
                    store.session("owner").unwrap(),
                ];
                let before_records = [
                    store.records("target").unwrap(),
                    store.records("owner").unwrap(),
                ];
                let mut good = record(
                    if conflict == "same-batch-type" {
                        "existing"
                    } else {
                        "fresh"
                    },
                    2,
                );
                good.cwd = Some("synthetic-workspace".into());
                let mut bad = record("existing", 3);
                if conflict != "other-session" {
                    bad.record_type = crate::model::RecordType::User;
                }
                match invalid {
                    "platform" => bad.source_platform = Some(String::new()),
                    "evidence-source" => {
                        bad.surface_evidence = Some(crate::SurfaceEvidence {
                            source: "invalid/source".into(),
                            version: None,
                        })
                    }
                    _ => {
                        bad.surface_evidence = Some(crate::SurfaceEvidence {
                            source: "adapter".into(),
                            version: Some("invalid/version".into()),
                        })
                    }
                }
                assert!(
                    matches!(
                        store.upsert_records("target", &[good, bad], keep_content),
                        Err(crate::Error::InvalidInput(_))
                    ),
                    "{conflict}, {invalid}, keep_content={keep_content}"
                );
                assert_eq!(store.counts().unwrap(), before_counts);
                assert_eq!(
                    [
                        store.session("target").unwrap(),
                        store.session("owner").unwrap()
                    ],
                    before_sessions
                );
                assert_eq!(
                    [
                        store.records("target").unwrap(),
                        store.records("owner").unwrap()
                    ],
                    before_records
                );
            }
        }
    }
}

#[test]
fn hygiene_unknown_facts_and_known_exclusions_share_projection() {
    use crate::{measurement::Projection, model::RecordIdentity};
    let cases = [
        (json!({"message":{}}), None),
        (json!({"message":{"role":"user"}}), None),
        (
            json!({"message":{"content":[{"type":"text","text":"Hello"}]}}),
            None,
        ),
        (json!({"message":{"role":"assistant"}}), Some(false)),
        (json!({"isMeta":true,"message":{}}), Some(false)),
        (json!({"isSidechain":true,"message":{}}), Some(false)),
        (json!({"message":{"content":[]}}), Some(false)),
        (
            json!({"message":{"content":[{"type":"tool_result","content":"Synthetic"}]}}),
            Some(false),
        ),
        (
            json!({"message":{"role":"user","content":[{"type":"text","text":"Ordinary <command-name> in the middle"}]}}),
            Some(true),
        ),
        (
            json!({"message":{"role":"user","content":[{"type":"text","text":" \n<com"},{"type":"text","text":"mand-name>/synthetic"}]}}),
            Some(false),
        ),
    ];
    let mut store = Store::open_in_memory().unwrap();
    session(&mut store, "hygiene");
    for (index, (mut value, expected)) in cases.into_iter().enumerate() {
        value["uuid"] = json!(format!("hygiene-{index}"));
        // Explicit role is authoritative even when the record type disagrees.
        value["type"] = json!("assistant");
        let input: CanonicalRecord = serde_json::from_value(value).unwrap();
        store
            .upsert_records("hygiene", std::slice::from_ref(&input), false)
            .unwrap();
        let saved = store
            .records("hygiene")
            .unwrap()
            .into_iter()
            .find(|row| row.uuid == input.uuid.as_deref().unwrap())
            .unwrap();
        assert_eq!(saved.classification.is_human, expected, "case {index}");
        assert_eq!(
            Projection::from_canonical("hygiene", &input, &RecordIdentity::default()).unwrap(),
            Projection::from_stored(&saved).unwrap()
        );
        assert!(saved.content_json.is_none());
    }
}

#[test]
fn hygiene_legacy_null_requires_replay_and_preserves_conflicts_and_prefix_flags() {
    let mut store = Store::open_in_memory().unwrap();
    session(&mut store, "legacy-hygiene");
    let input: CanonicalRecord = serde_json::from_value(json!({"uuid":"human","type":"user","message":{"role":"user","content":[{"type":"text","text":" Hello"}]}})).unwrap();
    let command: CanonicalRecord = serde_json::from_value(json!({"uuid":"command","type":"user","message":{"role":"user","content":[{"type":"text","text":" <command-name>/synthetic"}]}})).unwrap();
    store
        .upsert_records("legacy-hygiene", &[input.clone(), command.clone()], false)
        .unwrap();
    store
        .connection
        .execute("UPDATE records SET is_human=NULL", [])
        .unwrap();
    assert!(
        store
            .records("legacy-hygiene")
            .unwrap()
            .iter()
            .all(|row| row.classification.is_human.is_none())
    );
    let stats = store
        .upsert_records("legacy-hygiene", &[input.clone(), command.clone()], false)
        .unwrap();
    assert_eq!((stats.inserted, stats.enriched), (0, 2));
    let stats = store
        .upsert_records("legacy-hygiene", &[input.clone(), command], false)
        .unwrap();
    assert_eq!((stats.inserted, stats.enriched, stats.ignored), (0, 0, 2));
    let rows = store.records("legacy-hygiene").unwrap();
    let command = rows.iter().find(|row| row.uuid == "command").unwrap();
    assert_eq!(command.classification.is_human, Some(false));
    assert_eq!(command.classification.is_command, Some(false));
    assert!(!command.has_conflict);
    let mut conflicting = input;
    conflicting.message.role = Some("assistant".into());
    store
        .upsert_records("legacy-hygiene", &[conflicting], false)
        .unwrap();
    let row = store
        .records("legacy-hygiene")
        .unwrap()
        .into_iter()
        .find(|row| row.uuid == "human")
        .unwrap();
    assert!(row.has_conflict);
    assert_eq!(row.classification.is_human, Some(true));
    assert_eq!(row.role.as_deref(), Some("user"));
    assert!(row.content_json.is_none());
}

#[test]
fn hygiene_legacy_null_does_not_acquire_classification_from_conflicting_role() {
    let mut store = Store::open_in_memory().unwrap();
    session(&mut store, "legacy-conflict");
    let mut input: CanonicalRecord = serde_json::from_value(json!({"uuid":"row","type":"user","message":{"role":"assistant","content":[{"type":"text","text":"Hello"}]}})).unwrap();
    store
        .upsert_records("legacy-conflict", std::slice::from_ref(&input), false)
        .unwrap();
    store
        .connection
        .execute("UPDATE records SET is_human=NULL", [])
        .unwrap();
    input.message.role = Some("user".into());
    store
        .upsert_records("legacy-conflict", &[input], false)
        .unwrap();
    let saved = store.records("legacy-conflict").unwrap().remove(0);
    assert!(saved.has_conflict);
    assert_eq!(saved.role.as_deref(), Some("assistant"));
    assert_eq!(saved.classification.is_human, None);
}

#[test]
fn hygiene_legacy_null_does_not_acquire_classification_from_conflicting_prefix() {
    let mut store = Store::open_in_memory().unwrap();
    session(&mut store, "legacy-prefix");
    let text = "<command-name>/synthetic";
    let mut input: CanonicalRecord = serde_json::from_value(json!({"uuid":"row","type":"user","message":{"role":"user","content":[{"type":"text","text":text}]}})).unwrap();
    store
        .upsert_records("legacy-prefix", std::slice::from_ref(&input), false)
        .unwrap();
    store
        .connection
        .execute("UPDATE records SET is_human=NULL", [])
        .unwrap();
    input.message.content = Some(vec![json!({"type":"text","text":"x".repeat(text.len())})]);
    store
        .upsert_records("legacy-prefix", &[input], false)
        .unwrap();
    let saved = store.records("legacy-prefix").unwrap().remove(0);
    assert!(saved.has_conflict);
    assert_eq!(saved.classification.is_command, Some(true));
    assert_eq!(saved.classification.is_human, None);
}

#[test]
fn hygiene_legacy_null_metadata_conflicts_wait_for_compatible_replay() {
    for sidechain in [false, true] {
        let mut store = Store::open_in_memory().unwrap();
        session(&mut store, "legacy-flags");
        let original: CanonicalRecord = serde_json::from_value(json!({"uuid":"row","type":"user","message":{"role":"user","content":[{"type":"text","text":"Hello"}]}})).unwrap();
        store
            .upsert_records("legacy-flags", std::slice::from_ref(&original), false)
            .unwrap();
        store
            .connection
            .execute("UPDATE records SET is_human=NULL", [])
            .unwrap();
        let mut bad = original.clone();
        if sidechain {
            bad.is_sidechain = true;
        } else {
            bad.is_meta = true;
        }
        store.upsert_records("legacy-flags", &[bad], false).unwrap();
        let saved = store.records("legacy-flags").unwrap().remove(0);
        assert!(saved.has_conflict);
        assert_eq!(saved.classification.is_human, None);
        let stats = store
            .upsert_records("legacy-flags", &[original], false)
            .unwrap();
        assert_eq!(stats.enriched, 1);
        let saved = store.records("legacy-flags").unwrap().remove(0);
        assert!(saved.has_conflict);
        assert_eq!(saved.classification.is_human, Some(true));
    }
}

#[test]
fn hygiene_compatible_classification_enriches_despite_unrelated_conflicts() {
    let mut store = Store::open_in_memory().unwrap();
    session(&mut store, "unrelated");
    let mut input: CanonicalRecord = serde_json::from_value(json!({"uuid":"row","type":"user","timestamp":"2026-09-07T00:00:00Z","message":{"role":"user","model":"original","content":[{"type":"text","text":"Hello"}],"usage":{"input_tokens":10}}})).unwrap();
    store
        .upsert_records("unrelated", std::slice::from_ref(&input), false)
        .unwrap();
    store
        .connection
        .execute("UPDATE records SET is_human=NULL", [])
        .unwrap();
    input.timestamp = Some("2026-09-07T00:00:01Z".into());
    input.message.model = Some("conflicting".into());
    input.message.usage.as_mut().unwrap().input_tokens = Some(20);
    store.upsert_records("unrelated", &[input], false).unwrap();
    let saved = store.records("unrelated").unwrap().remove(0);
    assert!(saved.has_conflict);
    assert_eq!(saved.classification.is_human, Some(true));
    assert_eq!(saved.model.as_deref(), Some("original"));
    assert_eq!(saved.usage.unwrap().input_tokens, Some(10));
}
