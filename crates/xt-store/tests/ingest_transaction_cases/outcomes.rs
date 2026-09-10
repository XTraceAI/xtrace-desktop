use super::support::*;
use serde_json::json;
use xt_fixtures::TempDb;
use xt_store::{
    SessionSource, WriteStats,
    batch::{IngestBatch, RecordDisposition, SubmittedReceipt},
    model::RecordType,
};

#[test]
fn per_input_indices_disambiguate_duplicates_enrichment_conflicts_and_rejections() {
    let mut db = TempDb::empty().unwrap();
    db.store_mut()
        .upsert_session(&session("foreign"), true)
        .unwrap();
    db.store_mut()
        .upsert_records("foreign", &[rich("foreign-record")], true)
        .unwrap();
    let mut conflicting = rich("same");
    conflicting.message.usage.as_mut().unwrap().input_tokens = Some(99);
    conflicting.message.usage.as_mut().unwrap().output_tokens = Some(7);
    let mut wrong_type = rich("same");
    wrong_type.record_type = RecordType::User;
    let mut missing = rich(" ");
    missing.timestamp = Some("invalid".into());
    missing.message.usage.as_mut().unwrap().input_tokens = Some(-1);
    let rows = [
        record("same"),
        rich("same"),
        conflicting,
        record("same"),
        wrong_type,
        rich("same"),
        rich("foreign-record"),
        missing,
    ];
    let metadata = session("target");
    let output = db
        .store_mut()
        .apply_ingest_batch(&IngestBatch::new(&metadata, &rows, true))
        .unwrap();
    assert_eq!(
        output.stats,
        WriteStats {
            inserted: 1,
            enriched: 2,
            ignored: 4,
            dropped_no_uuid: 1
        }
    );
    assert_eq!(
        output
            .records
            .iter()
            .map(|row| row.input_index)
            .collect::<Vec<_>>(),
        (0..8).collect::<Vec<_>>()
    );
    assert_eq!(
        output
            .records
            .iter()
            .map(|row| row.disposition)
            .collect::<Vec<_>>(),
        [
            RecordDisposition::Inserted,
            RecordDisposition::Enriched,
            RecordDisposition::Enriched,
            RecordDisposition::Duplicate,
            RecordDisposition::RejectedType,
            RecordDisposition::Duplicate,
            RecordDisposition::RejectedOwnership,
            RecordDisposition::DroppedMissingUuid
        ]
    );
    assert_eq!(
        output
            .records
            .iter()
            .map(|row| row.stored_has_conflict)
            .collect::<Vec<_>>(),
        [
            Some(false),
            Some(false),
            Some(true),
            Some(true),
            Some(true),
            Some(true),
            Some(true),
            None
        ]
    );
    assert_eq!(
        output
            .records
            .iter()
            .map(|row| row.uuid.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("same"),
            Some("same"),
            Some("same"),
            Some("same"),
            Some("same"),
            Some("same"),
            Some("foreign-record"),
            None
        ]
    );
    let stored = db.store().records("target").unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].usage.as_ref().unwrap().input_tokens, Some(10));
    assert_eq!(stored[0].usage.as_ref().unwrap().output_tokens, Some(7));
    assert_eq!(stored[0].tool_uses.len(), 1);
    assert_eq!(
        db.store().records("foreign").unwrap()[0].session_id,
        "foreign"
    );
    // Legacy calls keep their original per-input counters and use the same merger.
    assert_eq!(
        db.store_mut()
            .upsert_records("target", &rows, true)
            .unwrap(),
        WriteStats {
            ignored: 7,
            dropped_no_uuid: 1,
            ..WriteStats::default()
        }
    );
}

#[test]
fn cursor_scope_monotonicity_and_empty_batches_are_explicit() {
    let mut db = TempDb::empty().unwrap();
    let metadata = session("target");
    for next in [
        cursor(SessionSource::Transcript, "same-key", 20),
        cursor(SessionSource::ReadersCli, "same-key", 30),
        cursor(SessionSource::Transcript, "other-key", 40),
    ] {
        let mut batch = IngestBatch::new(&metadata, &[], false);
        batch.cursor = Some(&next);
        let outcome = db.store_mut().apply_ingest_batch(&batch).unwrap();
        assert!(outcome.records.is_empty());
        assert_eq!(outcome.stats, WriteStats::default());
    }
    assert!(
        db.store()
            .source_cursor(SessionSource::Plugin, "same-key")
            .unwrap()
            .is_none()
    );
    let mut next = cursor(SessionSource::Transcript, "same-key", 20);
    next.updated_at = 50;
    let mut batch = IngestBatch::new(&metadata, &[], false);
    batch.cursor = Some(&next);
    db.store_mut().apply_ingest_batch(&batch).unwrap();
    assert_eq!(
        db.store()
            .source_cursor(next.source, "same-key")
            .unwrap()
            .unwrap()
            .updated_at,
        100
    );
    for position in [-1, 19] {
        let before = snapshot(&sql(&db));
        let regressing = cursor(SessionSource::Transcript, "same-key", position);
        let rows = [rich("must-rollback")];
        let mut batch = IngestBatch::new(&metadata, &rows, true);
        batch.cursor = Some(&regressing);
        assert!(db.store_mut().apply_ingest_batch(&batch).is_err());
        assert_eq!(snapshot(&sql(&db)), before);
    }
    assert_eq!(
        db.store()
            .source_cursor(SessionSource::ReadersCli, "same-key")
            .unwrap()
            .unwrap()
            .position,
        30
    );
    assert_eq!(
        db.store()
            .source_cursor(SessionSource::Transcript, "other-key")
            .unwrap()
            .unwrap()
            .position,
        40
    );
    assert_eq!(db.store().counts().unwrap().records, 0);
    let (receipt, coverage) = receipt_case("target", "absent");
    let mut batch = IngestBatch::new(&metadata, &[], false);
    batch.receipt = Some(SubmittedReceipt {
        receipt: &receipt,
        coverage: &coverage,
    });
    assert!(db.store_mut().apply_ingest_batch(&batch).is_err());
    batch.receipt = Some(SubmittedReceipt {
        receipt: &receipt,
        coverage: &[],
    });
    assert!(db.store_mut().apply_ingest_batch(&batch).is_err());
    assert!(db.store().capture_receipts("target").unwrap().is_empty());
}

#[test]
fn explicit_retention_applies_to_fresh_and_enriched_batch_content() {
    let mut db = TempDb::empty().unwrap();
    let mut metadata = session("target");
    metadata.title = Some("synthetic retained title".into());
    db.store_mut()
        .apply_ingest_batch(&IngestBatch::new(
            &metadata,
            &[rich("kept"), record("poor")],
            true,
        ))
        .unwrap();
    let saved = db.store().records("target").unwrap();
    let retained = saved
        .iter()
        .find(|row| row.uuid == "kept")
        .unwrap()
        .content_json
        .clone();
    metadata.title = Some("new title must not replace retained content".into());
    let rows = [rich("kept"), rich("poor"), rich("fresh")];
    db.store_mut()
        .apply_ingest_batch(&IngestBatch::new(&metadata, &rows, false))
        .unwrap();
    for row in db.store().records("target").unwrap() {
        assert_eq!(row.usage.as_ref().unwrap().input_tokens, Some(10));
        assert_eq!(row.tool_use_count, Some(1));
        assert_eq!(row.text_len, Some(0));
        if row.uuid == "kept" {
            assert_eq!(row.content_json, retained);
        } else {
            assert_eq!(row.content_json, None);
            assert_eq!(row.tool_uses[0].input_json, None);
        }
    }
    assert_eq!(
        db.store()
            .session("target")
            .unwrap()
            .unwrap()
            .meta
            .title
            .as_deref(),
        Some("synthetic retained title")
    );
    let mut fresh = session("metadata");
    fresh.title = Some("synthetic omitted title".into());
    db.store_mut()
        .apply_ingest_batch(&IngestBatch::new(&fresh, &[rich("metadata")], false))
        .unwrap();
    assert_eq!(
        db.store().session("metadata").unwrap().unwrap().meta.title,
        None
    );
    let row = &db.store().records("metadata").unwrap()[0];
    assert_eq!(row.content_json, None);
    assert_eq!(row.tool_uses[0].input_json, None);
    assert_eq!(
        serde_json::to_value(row.usage.as_ref().unwrap()).unwrap()["cache_read_input_tokens"],
        json!(0)
    );
}
