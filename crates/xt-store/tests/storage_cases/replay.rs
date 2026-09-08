use super::support::*;
use serde_json::json;
use xt_store::{Error, SessionSource, WriteStats};

#[test]
fn uuid_replay_and_missing_fields() {
    let mut store = memory_store();
    let mut batch: Vec<_> = (0..100)
        .map(|n| record(&format!("synthetic-{n}")))
        .collect();
    let mut missing = record("omitted");
    missing.uuid = None;
    batch.push(missing);
    assert_eq!(
        store.upsert_records(SESSION, &batch, true).unwrap(),
        WriteStats {
            inserted: 100,
            dropped_no_uuid: 1,
            ..WriteStats::default()
        }
    );
    batch.reverse();
    assert_eq!(
        store.upsert_records(SESSION, &batch, true).unwrap(),
        WriteStats {
            ignored: 100,
            dropped_no_uuid: 1,
            ..WriteStats::default()
        }
    );
    assert_eq!(
        store
            .upsert_records(SESSION, &[rich_record("synthetic-50")], true)
            .unwrap(),
        WriteStats {
            enriched: 1,
            ..WriteStats::default()
        }
    );
    assert_eq!(
        store.upsert_records(SESSION, &batch, true).unwrap().ignored,
        100
    );
    let rows = store.records(SESSION).unwrap();
    let enriched = rows.iter().find(|r| r.uuid == "synthetic-50").unwrap();
    assert_eq!(enriched.ts_ms, Some(1_788_264_000_123));
    assert_eq!(enriched.model.as_deref(), Some("model-synthetic"));
    assert_eq!(enriched.api_message_id.as_deref(), Some("api-synthetic"));
    assert_eq!(enriched.request_id.as_deref(), Some("request-synthetic"));
    let usage = enriched.usage.as_ref().unwrap();
    assert_eq!(usage.cache_read_input_tokens, Some(0));
    assert_eq!(
        usage
            .cache_creation
            .as_ref()
            .unwrap()
            .ephemeral_1h_input_tokens,
        Some(3)
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.ts.is_none() && r.ts_ms.is_none())
            .count(),
        99
    );
    let stored_session = store.session(SESSION).unwrap().unwrap();
    assert_eq!(
        stored_session.meta.surface.as_deref(),
        Some("future-desktop.v2")
    );
    assert_eq!(
        stored_session.meta.native_session_id.as_deref(),
        Some("native-synthetic")
    );
    assert_eq!(
        stored_session.meta.surface_evidence.unwrap().source,
        "adapter-next"
    );
    assert_eq!(stored_session.meta.started_at_ms, None);
    assert_eq!(store.counts().unwrap().usage_rows, 1);
}

#[test]
fn both_source_arrival_orders_have_identical_non_conflicting_records() {
    let mut snapshots = Vec::new();
    for rich_first in [false, true] {
        let mut store = memory_store();
        let poor = record("arrival-order");
        let rich = rich_record("arrival-order");
        let mut metadata = session(SESSION);
        metadata.started_at_ms = Some(123);
        for (n, input) in if rich_first {
            [&rich, &poor]
        } else {
            [&poor, &rich]
        }
        .into_iter()
        .enumerate()
        {
            metadata.source = if n == 0 {
                SessionSource::ReadersCli
            } else {
                SessionSource::Plugin
            };
            store.upsert_session(&metadata, true).unwrap();
            let stats = store
                .upsert_records(SESSION, std::slice::from_ref(input), true)
                .unwrap();
            assert_eq!(stats.inserted, usize::from(n == 0));
        }
        assert_eq!(store.counts().unwrap().records, 1);
        assert_eq!(
            store.session(SESSION).unwrap().unwrap().meta.started_at_ms,
            Some(123)
        );
        let rows = store.records(SESSION).unwrap();
        assert!(!rows[0].has_conflict);
        snapshots.push(rows);
    }
    assert_eq!(snapshots[0], snapshots[1]);
}

#[test]
fn response_ids_do_not_collapse_distinct_content_records() {
    let mut store = memory_store();
    let mut batch: Vec<_> = (0..4)
        .map(|n| rich_record(&format!("snapshot-{n}")))
        .collect();
    let mut missing_api = rich_record("missing-api");
    missing_api.message.id = None;
    let mut missing_request = rich_record("missing-request");
    missing_request.request_id = None;
    batch.extend([missing_api, missing_request]);
    assert_eq!(
        store
            .upsert_records(SESSION, &batch, true)
            .unwrap()
            .inserted,
        6
    );
    batch.reverse();
    assert_eq!(
        store.upsert_records(SESSION, &batch, true).unwrap().ignored,
        6
    );
    assert_eq!(store.counts().unwrap().usage_rows, 6);
    assert_eq!(
        store
            .records(SESSION)
            .unwrap()
            .iter()
            .map(|r| r.tool_uses.len())
            .sum::<usize>(),
        6
    );
}

#[test]
fn conflicting_non_null_values_and_uuid_ownership_are_preserved() {
    let mut store = memory_store();
    store
        .upsert_records(SESSION, &[rich_record("conflict")], true)
        .unwrap();
    let mut conflicting = rich_record("conflict");
    conflicting.timestamp = Some("2026-09-02T00:00:00Z".into());
    conflicting.message.model = Some("conflicting-model".into());
    conflicting.message.usage.as_mut().unwrap().input_tokens = Some(999);
    assert_eq!(
        store
            .upsert_records(SESSION, &[conflicting.clone()], true)
            .unwrap()
            .ignored,
        1
    );
    let row = store.records(SESSION).unwrap().remove(0);
    assert!(row.has_conflict);
    assert_eq!(row.model.as_deref(), Some("model-synthetic"));
    assert_eq!(row.usage.unwrap().input_tokens, Some(10));
    assert_eq!(row.ts.as_deref(), Some("2026-09-01T12:00:00.123Z"));
    store
        .upsert_session(&session("another-session"), true)
        .unwrap();
    assert_eq!(
        store
            .upsert_records("another-session", &[conflicting], true)
            .unwrap()
            .ignored,
        1
    );
    assert!(store.records("another-session").unwrap().is_empty());
    assert_eq!(
        store
            .session("another-session")
            .unwrap()
            .unwrap()
            .meta
            .surface,
        None
    );
    assert_eq!(store.counts().unwrap().records, 1);
}

#[test]
fn partial_usage_keeps_unknowns_and_measured_zero() {
    let mut store = memory_store();
    let mut input = record("partial");
    input.message.usage = Some(serde_json::from_value(json!({"input_tokens":0})).unwrap());
    store.upsert_records(SESSION, &[input], false).unwrap();
    let partial = store.records(SESSION).unwrap().remove(0).usage.unwrap();
    assert_eq!(partial.input_tokens, Some(0));
    assert_eq!(partial.output_tokens, None);
    assert_eq!(partial.cache_read_input_tokens, None);
    let mut richer = record("partial");
    richer.message.usage = Some(
        serde_json::from_value(json!({"output_tokens":7,"cache_read_input_tokens":0})).unwrap(),
    );
    assert_eq!(
        store
            .upsert_records(SESSION, &[richer], false)
            .unwrap()
            .enriched,
        1
    );
    let enriched = store.records(SESSION).unwrap().remove(0).usage.unwrap();
    assert_eq!(enriched.input_tokens, Some(0));
    assert_eq!(enriched.output_tokens, Some(7));
    assert_eq!(enriched.cache_creation_input_tokens, None);
}

#[test]
fn invalid_batch_rolls_back_records_and_session_enrichment() {
    let mut store = memory_store();
    let good = rich_record("valid-before-error");
    let mut invalid = rich_record("invalid");
    invalid.message.usage.as_mut().unwrap().output_tokens = Some(-1);
    assert!(matches!(
        store.upsert_records(SESSION, &[good, invalid], false),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(store.counts().unwrap().records, 0);
    let metadata = store.session(SESSION).unwrap().unwrap();
    assert_eq!(metadata.meta.surface, None);
    assert_eq!(metadata.first_ts, None);
    let mut invalid_timestamp = record("bad-time");
    invalid_timestamp.timestamp = Some("not-a-timestamp".into());
    assert!(
        store
            .upsert_records(SESSION, &[invalid_timestamp], true)
            .is_err()
    );
}

#[test]
fn timestamp_range_uses_instants_and_does_not_invent_native_start() {
    let mut store = memory_store();
    let mut first = record("first");
    first.timestamp = Some("2026-09-01T10:00:00+02:00".into());
    let mut last = record("last");
    last.timestamp = Some("2026-09-01T09:00:00Z".into());
    store
        .upsert_records(SESSION, &[last, record("unknown"), first], true)
        .unwrap();
    let metadata = store.session(SESSION).unwrap().unwrap();
    assert_eq!(
        metadata.first_ts.as_deref(),
        Some("2026-09-01T10:00:00+02:00")
    );
    assert_eq!(metadata.last_ts.as_deref(), Some("2026-09-01T09:00:00Z"));
    assert_eq!(metadata.meta.started_at_ms, None);
    assert_eq!(
        store
            .records(SESSION)
            .unwrap()
            .iter()
            .map(|r| r.uuid.as_str())
            .collect::<Vec<_>>(),
        ["first", "last", "unknown"]
    );
}

#[test]
fn repeated_uuid_inside_one_batch_has_one_durable_identity() {
    let mut store = memory_store();
    let mut missing = record(" ");
    missing.uuid = None;
    let inputs = [
        record("same-batch"),
        rich_record("same-batch"),
        rich_record("same-batch"),
        record("  "),
        missing,
    ];
    assert_eq!(
        store.upsert_records(SESSION, &inputs, true).unwrap(),
        WriteStats {
            inserted: 1,
            enriched: 1,
            ignored: 1,
            dropped_no_uuid: 2,
        }
    );
    let rows = store.records(SESSION).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tool_uses.len(), 1);
    assert_eq!(store.counts().unwrap().usage_rows, 1);
}

#[test]
fn equivalent_timestamp_offsets_do_not_conflict_but_distinct_instants_do() {
    let mut store = memory_store();
    let mut input = record("offsets");
    input.timestamp = Some("2026-09-01T12:00:00.1231Z".into());
    store
        .upsert_records(SESSION, &[input.clone()], true)
        .unwrap();
    input.timestamp = Some("2026-09-01T14:00:00.1231+02:00".into());
    assert_eq!(
        store
            .upsert_records(SESSION, &[input.clone()], true)
            .unwrap()
            .ignored,
        1
    );
    let equivalent = store.records(SESSION).unwrap().remove(0);
    assert!(!equivalent.has_conflict);
    assert_eq!(equivalent.ts.as_deref(), Some("2026-09-01T12:00:00.1231Z"));
    input.timestamp = Some("2026-09-01T12:00:00.1232Z".into());
    assert_eq!(
        store
            .upsert_records(SESSION, &[input], true)
            .unwrap()
            .ignored,
        1
    );
    assert!(store.records(SESSION).unwrap()[0].has_conflict);
}
