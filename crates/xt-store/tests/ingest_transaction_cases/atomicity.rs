use super::support::*;
use xt_fixtures::TempDb;
use xt_store::{
    SessionSource, Store,
    batch::{IngestBatch, RecordDisposition, SubmittedReceipt},
    ingest::{RecordSourceObservation, SessionSourceObservation},
    model::RecordType,
};

#[test]
fn commits_supplied_facts_and_reopens_with_the_shared_fixture() {
    let f1 = fixture();
    let loaded = &f1.sessions()[0];
    let mut db = TempDb::empty().unwrap();
    let id = &loaded.metadata.session_id;
    let uuid = loaded.records[0].uuid.as_deref().unwrap();
    let session_sources = [SessionSourceObservation {
        session_id: id.clone(),
        source: SessionSource::Fixture,
        first_seen_at: 1,
        last_seen_at: 2,
    }];
    let record_sources = [RecordSourceObservation {
        uuid: uuid.into(),
        source: SessionSource::Fixture,
        field_presence: 1,
        conflict_flags: 0,
    }];
    let (receipt, coverage) = receipt_case(id, uuid);
    let cursor = cursor(SessionSource::Fixture, "f1-records", 25);
    let mut batch = IngestBatch::new(&loaded.metadata, &loaded.records, true);
    batch.session_sources = &session_sources;
    batch.record_sources = &record_sources;
    batch.receipt = Some(SubmittedReceipt {
        receipt: &receipt,
        coverage: &coverage,
    });
    batch.cursor = Some(&cursor);
    let outcome = db.store_mut().apply_ingest_batch(&batch).unwrap();
    assert_eq!(outcome.stats.inserted, 25);
    assert!(
        outcome
            .records
            .iter()
            .enumerate()
            .all(|(index, row)| row.input_index == index
                && row.uuid == loaded.records[index].uuid
                && row.disposition == RecordDisposition::Inserted)
    );
    let reopened = Store::open(db.path()).unwrap();
    assert_eq!(reopened.counts().unwrap().records, 25);
    assert_eq!(reopened.counts().unwrap().usage_rows, 15);
    assert_eq!(
        reopened
            .records(id)
            .unwrap()
            .iter()
            .map(|record| record.tool_uses.len())
            .sum::<usize>(),
        5
    );
    assert_eq!(reopened.session_sources(id).unwrap(), session_sources);
    assert_eq!(reopened.record_sources(uuid).unwrap(), record_sources);
    assert_eq!(
        reopened.capture_receipts(id).unwrap(),
        std::slice::from_ref(&receipt)
    );
    assert_eq!(
        reopened.capture_coverage("synthetic-receipt").unwrap(),
        coverage
    );
    assert_eq!(
        reopened
            .source_cursor(cursor.source, &cursor.cursor_key)
            .unwrap(),
        Some(cursor.clone())
    );
    let before = snapshot(&sql(&db));
    assert!(db.store_mut().apply_ingest_batch(&batch).is_err());
    assert_eq!(snapshot(&sql(&db)), before, "sealed ID reuse is atomic");
}

#[test]
fn mixed_accepted_rejected_occurrences_cannot_supply_uuid_level_evidence() {
    let mut db = TempDb::empty().unwrap();
    let original = session("target");
    db.store_mut()
        .apply_ingest_batch(&IngestBatch::new(&original, &[rich("same")], true))
        .unwrap();
    let mut metadata = original;
    metadata.title = Some("must roll back with ambiguous evidence".into());
    let mut rejected = rich("same");
    rejected.record_type = RecordType::User;
    let (receipt, coverage) = receipt_case("target", "same");
    let sources = [RecordSourceObservation {
        uuid: "same".into(),
        source: SessionSource::Plugin,
        field_presence: 1,
        conflict_flags: 0,
    }];
    for rows in [[rich("same"), rejected.clone()], [rejected, rich("same")]] {
        for with_receipt in [false, true] {
            let before = snapshot(&sql(&db));
            let mut batch = IngestBatch::new(&metadata, &rows, true);
            if with_receipt {
                batch.receipt = Some(SubmittedReceipt {
                    receipt: &receipt,
                    coverage: &coverage,
                });
            } else {
                batch.record_sources = &sources;
            }
            assert!(db.store_mut().apply_ingest_batch(&batch).is_err());
            assert_eq!(snapshot(&sql(&db)), before);
        }
    }
    // A conflict in a known value still accepts the same canonical identity.
    // Its successful enrichment and submitted evidence remain eligible together.
    let mut conflicting = rich("same");
    conflicting.message.usage.as_mut().unwrap().input_tokens = Some(99);
    conflicting.message.usage.as_mut().unwrap().output_tokens = Some(7);
    let rows = [rich("same"), conflicting];
    let mut batch = IngestBatch::new(&metadata, &rows, true);
    batch.record_sources = &sources;
    batch.receipt = Some(SubmittedReceipt {
        receipt: &receipt,
        coverage: &coverage,
    });
    let outcome = db.store_mut().apply_ingest_batch(&batch).unwrap();
    assert!(
        outcome
            .records
            .iter()
            .all(|row| row.disposition.is_accepted())
    );
    assert_eq!(outcome.records[1].disposition, RecordDisposition::Enriched);
    assert_eq!(outcome.records[1].stored_has_conflict, Some(true));
    let saved = &db.store().records("target").unwrap()[0];
    assert_eq!(saved.usage.as_ref().unwrap().input_tokens, Some(10));
    assert_eq!(saved.usage.as_ref().unwrap().output_tokens, Some(7));
    assert_eq!(db.store().record_sources("same").unwrap(), sources);
    assert_eq!(db.store().capture_receipts("target").unwrap(), [receipt]);
    assert_eq!(
        db.store().capture_coverage("synthetic-receipt").unwrap(),
        coverage
    );
}

#[test]
fn every_late_failure_including_commit_rolls_back_before_retry() {
    for failure in [
        "session_sources",
        "record_sources",
        "capture_record_coverage",
        "source_cursors",
        "commit",
    ] {
        let mut db = TempDb::empty().unwrap();
        let original = session("target");
        let original_records = [record("existing")];
        let prior_cursor = cursor(SessionSource::Fixture, "tail", 10);
        let mut initial = IngestBatch::new(&original, &original_records, true);
        initial.cursor = Some(&prior_cursor);
        db.store_mut().apply_ingest_batch(&initial).unwrap();
        let connection = sql(&db);
        let before = snapshot(&connection);
        if failure == "commit" {
            connection.execute_batch("CREATE TABLE deferred_failure(session_id TEXT REFERENCES sessions(session_id) DEFERRABLE INITIALLY DEFERRED);
                CREATE TRIGGER inject_failure AFTER INSERT ON records BEGIN INSERT INTO deferred_failure VALUES ('absent-session'); END;").unwrap();
        } else {
            connection.execute_batch(&format!("CREATE TRIGGER inject_failure BEFORE INSERT ON {failure} BEGIN SELECT RAISE(ABORT,'synthetic transaction failure'); END;")).unwrap();
        }
        let mut metadata = original.clone();
        metadata.title = Some("synthetic title".into());
        let records = [rich("existing"), rich("new")];
        let sources = [SessionSourceObservation {
            session_id: "target".into(),
            source: SessionSource::Plugin,
            first_seen_at: 1,
            last_seen_at: 2,
        }];
        let record_sources = [RecordSourceObservation {
            uuid: "new".into(),
            source: SessionSource::Plugin,
            field_presence: 1,
            conflict_flags: 0,
        }];
        let (receipt, coverage) = receipt_case("target", "new");
        let next_cursor = cursor(SessionSource::Fixture, "tail", 20);
        let mut batch = IngestBatch::new(&metadata, &records, true);
        batch.session_sources = &sources;
        batch.record_sources = &record_sources;
        batch.receipt = Some(SubmittedReceipt {
            receipt: &receipt,
            coverage: &coverage,
        });
        batch.cursor = Some(&next_cursor);
        assert!(
            db.store_mut().apply_ingest_batch(&batch).is_err(),
            "{failure}"
        );
        assert_eq!(snapshot(&connection), before, "{failure}");
        assert_eq!(
            Store::open(db.path())
                .unwrap()
                .source_cursor(prior_cursor.source, "tail")
                .unwrap(),
            Some(prior_cursor)
        );
        assert!(db.store().capture_receipts("target").unwrap().is_empty());
        connection
            .execute_batch("DROP TRIGGER inject_failure")
            .unwrap();
        let result = db.store_mut().apply_ingest_batch(&batch).unwrap();
        assert_eq!((result.stats.inserted, result.stats.enriched), (1, 1));
        assert_eq!(db.store().counts().unwrap().records, 2);
        assert_eq!(db.store().capture_receipts("target").unwrap(), [receipt]);
        assert_eq!(
            db.store().capture_coverage("synthetic-receipt").unwrap(),
            coverage
        );
        assert_eq!(
            db.store()
                .source_cursor(next_cursor.source, "tail")
                .unwrap(),
            Some(next_cursor)
        );
        assert!(
            db.store()
                .session("target")
                .unwrap()
                .unwrap()
                .meta
                .title
                .is_some()
        );
    }
}

#[test]
fn supplied_evidence_must_match_the_session_and_accepted_submission() {
    let mut db = TempDb::empty().unwrap();
    for id in ["target", "foreign"] {
        db.store_mut().upsert_session(&session(id), true).unwrap();
        db.store_mut()
            .upsert_records(id, &[record(id)], true)
            .unwrap();
    }
    for wrong_uuid in ["target", "foreign", "missing"] {
        let metadata = session("target");
        let records = [record("submitted")];
        let (receipt, coverage) = receipt_case("target", wrong_uuid);
        let mut batch = IngestBatch::new(&metadata, &records, true);
        batch.receipt = Some(SubmittedReceipt {
            receipt: &receipt,
            coverage: &coverage,
        });
        let before = snapshot(&sql(&db));
        assert!(db.store_mut().apply_ingest_batch(&batch).is_err());
        assert_eq!(snapshot(&sql(&db)), before);
        let mut submitted_coverage = coverage.clone();
        submitted_coverage[0].record_uuid = "submitted".into();
        let sources = [RecordSourceObservation {
            uuid: wrong_uuid.into(),
            source: SessionSource::Fixture,
            field_presence: 1,
            conflict_flags: 0,
        }];
        batch.receipt = Some(SubmittedReceipt {
            receipt: &receipt,
            coverage: &submitted_coverage,
        });
        batch.record_sources = &sources;
        assert!(db.store_mut().apply_ingest_batch(&batch).is_err());
        assert_eq!(snapshot(&sql(&db)), before);
    }
    let metadata = session("target");
    let records = [record("foreign")];
    let (receipt, coverage) = receipt_case("target", "foreign");
    let mut batch = IngestBatch::new(&metadata, &records, true);
    batch.receipt = Some(SubmittedReceipt {
        receipt: &receipt,
        coverage: &coverage,
    });
    let before = snapshot(&sql(&db));
    assert!(db.store_mut().apply_ingest_batch(&batch).is_err());
    assert_eq!(
        snapshot(&sql(&db)),
        before,
        "rejected identity conflict flag also rolls back"
    );
    let wrong_session = [SessionSourceObservation {
        session_id: "foreign".into(),
        source: SessionSource::Fixture,
        first_seen_at: 1,
        last_seen_at: 1,
    }];
    batch.receipt = None;
    batch.session_sources = &wrong_session;
    assert!(db.store_mut().apply_ingest_batch(&batch).is_err());
    let (receipt, coverage) = receipt_case("foreign", "foreign");
    batch.session_sources = &[];
    batch.receipt = Some(SubmittedReceipt {
        receipt: &receipt,
        coverage: &coverage,
    });
    assert!(db.store_mut().apply_ingest_batch(&batch).is_err());
    assert_eq!(snapshot(&sql(&db)), before);
}

#[test]
fn malformed_rejected_records_roll_back_the_entire_composed_batch() {
    for owner in ["target", "foreign"] {
        for invalid_field in ["platform", "evidence", "timestamp", "usage"] {
            let mut db = TempDb::empty().unwrap();
            db.store_mut()
                .apply_ingest_batch(&IngestBatch::new(
                    &session(owner),
                    &[rich("existing")],
                    true,
                ))
                .unwrap();
            let before = snapshot(&sql(&db));
            let mut metadata = session("target");
            metadata.title = Some("synthetic batch title".into());
            let mut rejected = rich("existing");
            if owner == "target" {
                rejected.record_type = RecordType::User;
            }
            match invalid_field {
                "platform" => rejected.source_platform = Some(" ".into()),
                "evidence" => {
                    rejected.surface_evidence = Some(
                        serde_json::from_value(serde_json::json!({"source":"invalid label"}))
                            .unwrap(),
                    );
                }
                "timestamp" => rejected.timestamp = Some("invalid date".into()),
                "usage" => rejected.message.usage.as_mut().unwrap().input_tokens = Some(-1),
                _ => unreachable!(),
            }
            let rows = [rich("new"), rejected];
            let next = cursor(SessionSource::Fixture, "tail", 20);
            let mut batch = IngestBatch::new(&metadata, &rows, true);
            batch.cursor = Some(&next);
            assert!(
                db.store_mut().apply_ingest_batch(&batch).is_err(),
                "{owner}/{invalid_field}"
            );
            assert_eq!(snapshot(&sql(&db)), before, "{owner}/{invalid_field}");
            assert_eq!(db.store().source_cursor(next.source, "tail").unwrap(), None);
        }
    }
}
