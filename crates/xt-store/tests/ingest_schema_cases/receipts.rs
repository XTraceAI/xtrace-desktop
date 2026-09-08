use crate::support::*;
use rusqlite::params;
use serde_json::json;
use xt_store::{
    SessionMeta, SessionSource,
    ingest::{
        RecordCoverage, RecordSourceObservation, SessionSourceObservation, ToolEvent, ToolKind,
    },
};

#[test]
fn receipt_schema_keeps_distinct_facts() {
    let (mut db, case) = receipt_db(false);
    let session = &case.receipt.session_id;
    let uuid = &case.coverage[0].record_uuid;
    db.store_mut()
        .observe_session_source(&SessionSourceObservation {
            session_id: session.clone(),
            source: SessionSource::ReadersCli,
            first_seen_at: 1,
            last_seen_at: 2,
        })
        .unwrap();
    db.store_mut()
        .observe_record_source(&RecordSourceObservation {
            uuid: uuid.clone(),
            source: SessionSource::ReadersCli,
            field_presence: 7,
            conflict_flags: 0,
        })
        .unwrap();
    assert!(db.store().capture_receipts(session).unwrap().is_empty());
    db.store_mut()
        .insert_capture_receipt(&case.receipt, &case.coverage)
        .unwrap();
    assert_eq!(
        db.store().capture_receipts(session).unwrap(),
        std::slice::from_ref(&case.receipt)
    );
    assert_eq!(
        db.store()
            .capture_coverage(&case.receipt.receipt_id)
            .unwrap(),
        case.coverage
    );
    // Stored usage exists, but the supplied one-bit receipt never borrows it.
    assert!(db.store().records(session).unwrap()[0].usage.is_some());
    assert_eq!(
        db.store()
            .capture_coverage(&case.receipt.receipt_id)
            .unwrap()[0]
            .metric_field_mask,
        case.expected["metric_field_mask"].as_i64().unwrap()
    );
    let sql = connection(db.path());
    for (table, key) in [
        ("sessions", "sessions"),
        ("records", "records"),
        ("capture_receipts", "receipts"),
        ("capture_record_coverage", "covered_records"),
    ] {
        assert_eq!(
            scalar(&sql, &format!("SELECT count(*) FROM {table}")),
            case.expected[key].as_i64().unwrap()
        );
    }
    assert_eq!(
        db.store()
            .session(session)
            .unwrap()
            .unwrap()
            .meta
            .native_session_id
            .as_deref(),
        Some("native-fixture-18")
    );
    assert_eq!(
        db.store().session(session).unwrap().unwrap().meta.surface,
        case.receipt.surface
    );
    assert_eq!(db.store().session_sources(session).unwrap().len(), 1);
    assert_eq!(
        db.store().record_sources(uuid).unwrap()[0].field_presence,
        7
    );
    assert_eq!(
        scalar(&sql, "SELECT count(*) FROM pragma_foreign_key_check"),
        0
    );
}

#[test]
fn invalid_or_duplicate_coverage_rolls_back_the_whole_receipt() {
    let (mut db, case) = receipt_db(false);
    let mut bad_cases = vec![
        Vec::new(),
        vec![case.coverage[0].clone(), case.coverage[0].clone()],
    ];
    for field in [
        "uuid",
        "digest-short",
        "digest-upper",
        "digest-character",
        "version",
        "mask",
    ] {
        let mut item = case.coverage[0].clone();
        item.record_uuid = case.records[1].uuid.clone().unwrap();
        match field {
            "uuid" => item.record_uuid = "missing".into(),
            "digest-short" => item.measurement_revision = "a".repeat(63),
            "digest-upper" => item.measurement_revision = "A".repeat(64),
            "digest-character" => item.measurement_revision = "z".repeat(64),
            "version" => item.digest_schema_version = 0,
            "mask" => item.metric_field_mask = -1,
            _ => unreachable!(),
        }
        // A valid first child must roll back if any later submitted child fails.
        bad_cases.push(vec![case.coverage[0].clone(), item]);
    }
    db.store_mut()
        .upsert_session(
            &SessionMeta::new("other", "cursor", SessionSource::Fixture),
            false,
        )
        .unwrap();
    let mut other = case.records[0].clone();
    other.uuid = Some("other-record".into());
    db.store_mut()
        .upsert_records("other", &[other], false)
        .unwrap();
    let mut cross = case.coverage[0].clone();
    cross.record_uuid = "other-record".into();
    bad_cases.push(vec![cross]);
    for invalid in bad_cases {
        assert!(
            db.store_mut()
                .insert_capture_receipt(&case.receipt, &invalid)
                .is_err()
        );
        assert!(
            db.store()
                .capture_receipts(&case.receipt.session_id)
                .unwrap()
                .is_empty()
        );
        let sql = connection(db.path());
        assert_eq!(scalar(&sql, "SELECT count(*) FROM capture_receipts"), 0);
        assert_eq!(
            scalar(&sql, "SELECT count(*) FROM capture_record_coverage"),
            0
        );
    }
    db.store_mut()
        .insert_capture_receipt(&case.receipt, &case.coverage)
        .unwrap();
    assert!(
        db.store_mut()
            .insert_capture_receipt(&case.receipt, &case.coverage)
            .is_err()
    );
    assert_eq!(
        db.store()
            .capture_coverage(&case.receipt.receipt_id)
            .unwrap(),
        case.coverage
    );
}

#[test]
fn sealed_receipts_cannot_change_append_or_replace() {
    let (mut db, case) = receipt_db(false);
    db.store_mut()
        .insert_capture_receipt(&case.receipt, &case.coverage)
        .unwrap();
    let sql = connection(db.path());
    for statement in [
        "UPDATE capture_receipts SET received_at=0",
        "UPDATE capture_receipts SET coverage_sealed=0",
        "DELETE FROM capture_receipts",
        "UPDATE capture_record_coverage SET metric_field_mask=7",
        "DELETE FROM capture_record_coverage",
        "INSERT OR REPLACE INTO capture_receipts(receipt_id,session_id,surface,received_at) SELECT receipt_id,session_id,surface,0 FROM capture_receipts",
    ] {
        assert!(
            sql.execute(statement, []).is_err(),
            "immutable statement accepted: {statement}"
        );
    }
    assert!(sql.execute("INSERT INTO capture_record_coverage(receipt_id,record_uuid,session_id,metric_field_mask,measurement_revision,digest_schema_version) VALUES(?1,?2,?3,1,?4,1)",params![case.receipt.receipt_id,case.records[1].uuid,case.receipt.session_id,case.coverage[0].measurement_revision]).is_err());
    assert!(sql.execute("INSERT INTO capture_receipts(receipt_id,session_id,received_at,coverage_sealed) VALUES('presealed',?1,0,1)",[&case.receipt.session_id]).is_err());
    // New canonical observations cannot mutate coverage of the earlier payload.
    let mut enriched = case.records[0].clone();
    enriched
        .message
        .usage
        .as_mut()
        .unwrap()
        .cache_creation_input_tokens = Some(5);
    db.store_mut()
        .upsert_records(&case.receipt.session_id, &[enriched], false)
        .unwrap();
    assert_eq!(
        db.store()
            .capture_receipts(&case.receipt.session_id)
            .unwrap(),
        [case.receipt]
    );
    assert_eq!(
        db.store().capture_coverage("fixture-18-receipt").unwrap(),
        case.coverage
    );
}

#[test]
fn sql_digest_types_and_unsealed_visibility_are_enforced() {
    let (db, case) = receipt_db(false);
    let sql = connection(db.path());
    sql.execute(
        "INSERT INTO capture_receipts(receipt_id,session_id,received_at) VALUES('draft',?1,0)",
        [&case.receipt.session_id],
    )
    .unwrap();
    assert!(
        db.store()
            .capture_receipts(&case.receipt.session_id)
            .unwrap()
            .is_empty()
    );
    assert!(
        sql.execute(
            "UPDATE capture_receipts SET coverage_sealed=1 WHERE receipt_id='draft'",
            []
        )
        .is_err()
    );
    for (mask, digest, version) in [
        (json!(1.5), json!("a".repeat(64)), json!(1)),
        (json!(1), json!("a".repeat(64)), json!(1.5)),
    ] {
        assert!(sql.execute("INSERT INTO capture_record_coverage VALUES('draft',?1,?2,json_extract(?3,'$'),json_extract(?4,'$'),json_extract(?5,'$'))",params![case.coverage[0].record_uuid,case.receipt.session_id,mask.to_string(),digest.to_string(),version.to_string()]).is_err());
    }
    assert!(
        sql.execute(
            "INSERT INTO capture_record_coverage VALUES('draft',?1,?2,1,?3,1)",
            params![
                case.coverage[0].record_uuid,
                case.receipt.session_id,
                vec![b'a'; 64]
            ]
        )
        .is_err()
    );
    assert!(db.store().capture_coverage("draft").unwrap().is_empty());
}

#[test]
fn metadata_mode_keeps_structural_facts_without_new_content() {
    let (mut db, case) = receipt_db(false);
    let fixture = fixture("F18");
    let mut metadata = fixture.sessions()[0].metadata.clone();
    metadata.title = Some("Synthetic title must stay absent".into());
    db.store_mut().upsert_session(&metadata, false).unwrap();
    db.store_mut()
        .upsert_records(&metadata.session_id, &case.records, false)
        .unwrap();
    let event = ToolEvent {
        session_id: metadata.session_id.clone(),
        source: SessionSource::Plugin,
        source_event_id: "hook-1".into(),
        timestamp: None,
        name: "PreToolUse".into(),
        kind: ToolKind::Hook,
        server: None,
        tool: Some("Read".into()),
        skill: None,
    };
    let id = db.store_mut().insert_tool_event(&event).unwrap();
    assert!(id > 0);
    assert_eq!(
        db.store().tool_events(&metadata.session_id).unwrap(),
        std::slice::from_ref(&event)
    );
    assert!(db.store_mut().insert_tool_event(&event).is_err());
    db.store_mut()
        .insert_capture_receipt(&case.receipt, &case.coverage)
        .unwrap();
    let sql = connection(db.path());
    assert_eq!(
        scalar(
            &sql,
            "SELECT count(*) FROM records WHERE content_json IS NOT NULL"
        ),
        0
    );
    assert_eq!(
        scalar(
            &sql,
            "SELECT count(*) FROM sessions WHERE title IS NOT NULL"
        ),
        0
    );
    assert_eq!(
        scalar(
            &sql,
            "SELECT count(*) FROM tool_uses WHERE input_json IS NOT NULL"
        ),
        0
    );
    assert_eq!(scalar(&sql, "SELECT count(*) FROM records"), 2);
    assert_eq!(scalar(&sql, "SELECT record_count FROM sessions"), 2);
    assert_eq!(
        scalar(&sql, "SELECT count(*) FROM tool_uses WHERE uuid IS NULL"),
        1
    );
    for mut payload in [
        serde_json::to_value(&event).unwrap(),
        serde_json::to_value(&case.coverage[0]).unwrap(),
    ] {
        payload["raw_content"] = json!("must not enter structural input");
        if payload.get("record_uuid").is_some() {
            assert!(serde_json::from_value::<RecordCoverage>(payload).is_err());
        } else {
            assert!(serde_json::from_value::<ToolEvent>(payload).is_err());
        }
    }
    assert!(
        sql.execute(
            "UPDATE tool_uses SET input_json='{}' WHERE uuid IS NULL",
            []
        )
        .is_err()
    );
}

#[test]
fn structural_event_time_is_native_nullable_and_separate_from_record_time() {
    let (mut db, case) = receipt_db(false);
    let records_before = db.store().records(&case.receipt.session_id).unwrap();
    let event = ToolEvent {
        session_id: case.receipt.session_id.clone(),
        source: SessionSource::Plugin,
        source_event_id: "native-summary".into(),
        timestamp: Some("1969-12-31T18:59:59.999999999123-05:00".into()),
        name: "stop_hook_summary".into(),
        kind: ToolKind::Hook,
        server: None,
        tool: None,
        skill: None,
    };
    db.store_mut().insert_tool_event(&event).unwrap();
    let mut unknown = event.clone();
    unknown.source_event_id = "unknown-summary".into();
    unknown.timestamp = None;
    db.store_mut().insert_tool_event(&unknown).unwrap();
    assert!(
        db.store()
            .session(&event.session_id)
            .unwrap()
            .unwrap()
            .meta
            .started_at_ms
            .is_some()
    );
    assert_eq!(
        db.store().tool_events(&event.session_id).unwrap(),
        [event.clone(), unknown]
    );
    for invalid in ["", "not-a-timestamp", "2026-02-30T00:00:00Z"] {
        let mut invalid_event = event.clone();
        invalid_event.source_event_id = "invalid-summary".into();
        invalid_event.timestamp = Some(invalid.into());
        assert!(db.store_mut().insert_tool_event(&invalid_event).is_err());
        assert_eq!(db.store().tool_events(&event.session_id).unwrap().len(), 2);
    }
    let sql = connection(db.path());
    assert_eq!(
        sql.query_row(
            "SELECT event_ts FROM tool_uses WHERE source_event_id='native-summary'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        event.timestamp.unwrap()
    );
    assert_eq!(
        scalar(
            &sql,
            "SELECT count(*) FROM tool_uses WHERE uuid IS NOT NULL AND event_ts IS NOT NULL"
        ),
        0
    );
    assert_eq!(
        db.store().records(&case.receipt.session_id).unwrap(),
        records_before
    );
}
