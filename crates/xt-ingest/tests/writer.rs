use serde_json::{Value, json};
use xt_fixtures::TempDb;
use xt_ingest::{
    canonical::{Parsed, ParsedRecord, SourceContext, parse_line},
    writer::{WriteBatch, coverage, matches_current, write_batch},
};
use xt_store::{SessionSource, Store, batch::SourceCursor, ingest::CaptureReceipt};

fn parsed(value: Value) -> ParsedRecord {
    match parse_line(&value.to_string()).unwrap() {
        Parsed::Record(record) => *record,
        _ => panic!("expected synthetic record"),
    }
}
fn poor(uuid: &str) -> ParsedRecord {
    parsed(json!({"uuid":uuid,"type":"assistant","message":{}}))
}
fn rich(uuid: &str) -> ParsedRecord {
    parsed(
        json!({"uuid":uuid,"type":"assistant","parentUuid":"parent","agentId":"synthetic-agent","subtype":"assistant","timestamp":"2026-09-07T01:00:00.123456789012Z","message":{"role":"assistant","model":"synthetic-model","content":[{"type":"text","text":"Synthetic text"},{"type":"tool_use","name":"Read","input":{"secret_text":"never evidence"}}],"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"service_tier":"standard"}}}),
    )
}
fn context(source: SessionSource) -> SourceContext {
    SourceContext {
        conversation_id: Some("cursor-native".into()),
        native_session_id: Some("native".into()),
        source_platform: Some("cursor".into()),
        source_surface: Some("cli".into()),
        source: Some(source),
        ..Default::default()
    }
}
fn receipt(id: &str) -> CaptureReceipt {
    CaptureReceipt {
        receipt_id: id.into(),
        session_id: "cursor-native".into(),
        surface: Some("cli".into()),
        received_at: 100,
    }
}
fn request<'a>(
    context: &'a SourceContext,
    records: &'a [ParsedRecord],
    receipt: Option<&'a CaptureReceipt>,
) -> WriteBatch<'a> {
    WriteBatch {
        namespace: None,
        context,
        declared_host: None,
        records,
        title: Some("synthetic title"),
        cwd: None,
        git_branch: None,
        keep_content: false,
        observed_at: 100,
        receipt,
        cursor: None,
    }
}

#[test]
fn writer_source_orders_unique_counts_and_native_identity_agree() {
    let mut outputs = Vec::new();
    for plugin_first in [false, true] {
        let mut db = TempDb::empty().unwrap();
        for (source, rows) in if plugin_first {
            [
                (SessionSource::Plugin, vec![poor("row")]),
                (SessionSource::ReadersCli, vec![rich("row")]),
            ]
        } else {
            [
                (SessionSource::ReadersCli, vec![rich("row")]),
                (SessionSource::Plugin, vec![poor("row")]),
            ]
        } {
            let context = context(source);
            let receipt = receipt("capture");
            let output = write_batch(
                db.store_mut(),
                &request(
                    &context,
                    &rows,
                    (source == SessionSource::Plugin).then_some(&receipt),
                ),
            )
            .unwrap();
            assert_eq!(
                output.records_new,
                usize::from(db.store().session_sources("cursor-native").unwrap().len() == 1)
            );
            if output.records_new == 0 && source == SessionSource::ReadersCli {
                assert_eq!(output.records_enriched, 1);
                assert!(output.events[0].invalidate_cost);
            }
        }
        let row = db.store().records("cursor-native").unwrap().remove(0);
        assert_eq!(row.identity.agent_id.as_deref(), Some("synthetic-agent"));
        assert_eq!(row.usage.as_ref().unwrap().input_tokens, Some(10));
        assert!(row.content_json.is_none());
        assert!(row.tool_uses[0].input_json.is_none());
        assert_eq!(
            db.store()
                .session("cursor-native")
                .unwrap()
                .unwrap()
                .meta
                .title,
            None
        );
        let old = db.store().capture_coverage("capture").unwrap();
        assert!(!matches_current(&old[0], &row).unwrap());
        outputs.push(row);
    }
    // SQLite surrogate IDs are local; both isolated databases happen to assign1.
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
fn capture_receipts_exact_retry_full_replay_and_late_enrichment() {
    let mut db = TempDb::empty().unwrap();
    let plugin = context(SessionSource::Plugin);
    let native = context(SessionSource::ReadersCli);
    let rows = [poor("old"), poor("late")];
    write_batch(db.store_mut(), &request(&native, &rows, None)).unwrap();
    let late = [poor("late")];
    let submitted = receipt("late-only");
    let first = write_batch(db.store_mut(), &request(&plugin, &late, Some(&submitted))).unwrap();
    assert_eq!(first.records_new, 0);
    assert_eq!(first.records_enriched, 0);
    assert!(
        first.events[0].invalidate_measurements,
        "new receipt changes capture inputs even for duplicate-only rows"
    );
    assert_eq!(first.ack_through.as_deref(), Some("late"));
    assert_eq!(db.store().capture_coverage("late-only").unwrap().len(), 1);
    let again = write_batch(db.store_mut(), &request(&plugin, &late, Some(&submitted))).unwrap();
    assert_eq!(again.records_new, 0);
    assert_eq!(again.records_enriched, 0);
    assert!(again.events[0].invalidate_measurements);
    assert_eq!(again.ack_through, first.ack_through);
    assert_eq!(
        db.store().capture_receipts("cursor-native").unwrap().len(),
        1
    );
    let previous = db.store().capture_coverage("late-only").unwrap();
    let cached = db.store().records("cursor-native").unwrap();
    let before_row = cached.iter().find(|row| row.uuid == "late").unwrap();
    assert!(matches_current(&previous[0], before_row).unwrap());
    let cached_inputs = (before_row.usage.clone(), before_row.model.clone());
    let changed = write_batch(db.store_mut(), &request(&native, &[rich("late")], None)).unwrap();
    assert_eq!((changed.records_new, changed.records_enriched), (0, 1));
    assert!(changed.events[0].invalidate_measurements);
    assert!(changed.events[0].invalidate_cost);
    assert_eq!(db.store().capture_coverage("late-only").unwrap(), previous);
    let current = db.store().records("cursor-native").unwrap();
    let refreshed = current.iter().find(|row| row.uuid == "late").unwrap();
    assert_ne!(
        cached_inputs,
        (refreshed.usage.clone(), refreshed.model.clone())
    );
    assert_eq!(refreshed.usage.as_ref().unwrap().output_tokens, Some(5));
    assert!(
        !matches_current(
            &previous[0],
            current.iter().find(|row| row.uuid == "late").unwrap()
        )
        .unwrap()
    );
    let complete = [poor("old"), rich("late")];
    let full = receipt("full");
    write_batch(db.store_mut(), &request(&plugin, &complete, Some(&full))).unwrap();
    for evidence in db.store().capture_coverage("full").unwrap() {
        assert!(
            matches_current(
                &evidence,
                current
                    .iter()
                    .find(|row| row.uuid == evidence.record_uuid)
                    .unwrap()
            )
            .unwrap()
        );
    }
}

#[test]
fn capture_receipts_failure_and_mismatched_retry_roll_back_ack_cursor_and_rows() {
    for commit_failure in [false, true] {
        let mut db = TempDb::empty().unwrap();
        let plugin = context(SessionSource::Plugin);
        let receipt = receipt("stable");
        write_batch(
            db.store_mut(),
            &request(&plugin, &[poor("row")], Some(&receipt)),
        )
        .unwrap();
        let before = db.store().records("cursor-native").unwrap();
        let sql = rusqlite::Connection::open(db.path()).unwrap();
        let mut candidate = receipt.clone();
        if commit_failure {
            candidate.receipt_id = "new-receipt".into();
            sql.execute_batch("CREATE TABLE deferred_failure(session TEXT REFERENCES sessions(session_id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER injected AFTER INSERT ON records BEGIN INSERT INTO deferred_failure VALUES('missing'); END;").unwrap();
        }
        let rows = [rich("row"), rich("new")];
        let cursor = SourceCursor {
            source: SessionSource::Plugin,
            cursor_key: "delivery".into(),
            position: 2,
            updated_at: 100,
        };
        let mut batch = request(&plugin, &rows, Some(&candidate));
        batch.cursor = Some(&cursor);
        assert!(write_batch(db.store_mut(), &batch).is_err());
        assert_eq!(db.store().records("cursor-native").unwrap(), before);
        assert!(
            db.store()
                .source_cursor(SessionSource::Plugin, "delivery")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            db.store().capture_receipts("cursor-native").unwrap(),
            [receipt]
        );
        if commit_failure {
            sql.execute_batch("DROP TRIGGER injected").unwrap();
            let success = write_batch(db.store_mut(), &batch).unwrap();
            assert_eq!(success.ack_through.as_deref(), Some("new"));
            assert_eq!(success.records_new, 1);
        }
    }
}

#[test]
fn writer_unique_duplicates_rejections_and_source_conflict_masks() {
    let mut db = TempDb::empty().unwrap();
    let native = context(SessionSource::ReadersCli);
    let plugin = context(SessionSource::Plugin);
    let receipt = receipt("batch");
    let rows = [poor("same"), rich("same"), rich("same")];
    let output = write_batch(db.store_mut(), &request(&plugin, &rows, Some(&receipt))).unwrap();
    assert_eq!(
        (
            output.records_new,
            output.records_enriched,
            output.records_dropped
        ),
        (1, 0, 0)
    );
    assert_eq!(db.store().capture_coverage("batch").unwrap().len(), 1);
    let mut conflicting = rich("same");
    conflicting
        .canonical
        .message
        .usage
        .as_mut()
        .unwrap()
        .input_tokens = Some(99);
    write_batch(db.store_mut(), &request(&native, &[conflicting], None)).unwrap();
    let source = db
        .store()
        .record_sources("same")
        .unwrap()
        .into_iter()
        .find(|source| source.source == SessionSource::ReadersCli)
        .unwrap();
    assert_eq!(source.conflict_flags, 1 << 14);
    assert_eq!(
        db.store().records("cursor-native").unwrap()[0]
            .usage
            .as_ref()
            .unwrap()
            .input_tokens,
        Some(10)
    );
    let foreign_context = SourceContext {
        conversation_id: Some("foreign".into()),
        source: Some(SessionSource::ReadersCli),
        ..Default::default()
    };
    write_batch(
        db.store_mut(),
        &request(&foreign_context, &[poor("foreign")], None),
    )
    .unwrap();
    let submitted = receipt_clone("rejections");
    let rows = [poor("foreign"), rich("valid")];
    let output = write_batch(db.store_mut(), &request(&plugin, &rows, Some(&submitted))).unwrap();
    assert_eq!(output.records_dropped, 1);
    assert_eq!(output.ack_through.as_deref(), Some("valid"));
    assert_eq!(
        db.store()
            .capture_coverage("rejections")
            .unwrap()
            .iter()
            .map(|row| row.record_uuid.as_str())
            .collect::<Vec<_>>(),
        ["valid"]
    );
    db.store_mut()
        .upsert_records("cursor-native", &[poor("legacy").canonical], false)
        .unwrap();
    let legacy = [poor("legacy")];
    let mut batch = request(&native, &legacy, None);
    let output = write_batch(db.store_mut(), &batch).unwrap();
    assert_eq!(output.records_enriched, 1);
    batch.observed_at = 200;
    let output = write_batch(db.store_mut(), &batch).unwrap();
    assert_eq!(output.records_enriched, 0);
    let legacy = db
        .store()
        .records("cursor-native")
        .unwrap()
        .into_iter()
        .find(|row| row.uuid == "legacy")
        .unwrap();
    assert_eq!(legacy.identity.first_seen_at, Some(100));
    assert!(!legacy.has_conflict);
}
fn receipt_clone(id: &str) -> CaptureReceipt {
    receipt(id)
}

#[test]
fn writer_projection_is_versioned_content_free_nullable_and_precise() {
    let first = rich("row");
    let mut other = first.clone();
    other.canonical.timestamp = Some("2026-09-07T02:00:00.1234567890120+01:00".into());
    other.canonical.message.content.as_mut().unwrap()[0]["text"] = json!("Different text"); // same14 scalar length
    other.canonical.message.content.as_mut().unwrap()[1]["input"] = json!({"different":"private"});
    // Use same text length to prove only measurement structure enters evidence.
    other.canonical.message.content.as_mut().unwrap()[0]["text"] = json!("Other text xxx");
    assert_eq!(
        coverage("cursor-native", &first).unwrap(),
        coverage("cursor-native", &other).unwrap()
    );
    other.canonical.timestamp = Some("2026-09-07T01:00:00.123456789013Z".into());
    assert_ne!(
        coverage("cursor-native", &first).unwrap(),
        coverage("cursor-native", &other).unwrap()
    );
    let unknown = poor("row");
    let golden = coverage("cursor-native", &unknown).unwrap();
    assert_eq!(golden.metric_field_mask, 199);
    // Independently specified [version,28 fixed-order scalar/null values].
    assert_eq!(
        golden.measurement_revision,
        "0c299a8f1b8c32e9d61ebc26bedf0da9ac99dad88e889cce513f15698afcc795"
    );
    let mut zero = unknown.clone();
    zero.canonical.message.usage = Some(serde_json::from_value(json!({"input_tokens":0})).unwrap());
    assert_eq!(
        coverage("cursor-native", &zero).unwrap().metric_field_mask
            ^ coverage("cursor-native", &unknown)
                .unwrap()
                .metric_field_mask,
        1 << 14
    );
    let evidence = serde_json::to_string(&coverage("cursor-native", &first).unwrap()).unwrap();
    assert!(!evidence.contains("Synthetic text"));
    assert!(!evidence.contains("never evidence"));
    assert_eq!(
        coverage("cursor-native", &first)
            .unwrap()
            .measurement_revision
            .len(),
        64
    );
}

#[test]
fn writer_start_identity_compares_precise_instants_not_rfc3339_spelling() {
    for (left, right) in [
        ("2026-09-07T01:00:00Z", "2026-09-07T02:00:00+01:00"),
        (
            "2026-09-07T01:00:00.123456789012Z",
            "2026-09-07T02:00:00.1234567890120+01:00",
        ),
    ] {
        let mut db = TempDb::empty().unwrap();
        let mut native = context(SessionSource::Transcript);
        native.started_at = Some(left.into());
        let mut record = poor("start");
        record.context.started_at = Some(right.into());
        record.source.started_at = Some(left.into());
        write_batch(db.store_mut(), &request(&native, &[record.clone()], None)).unwrap();
        assert_eq!(
            db.store()
                .session("cursor-native")
                .unwrap()
                .unwrap()
                .meta
                .started_at_ms,
            Some(
                chrono::DateTime::parse_from_rfc3339(left)
                    .unwrap()
                    .timestamp_millis()
            )
        );
        let before = db.store().session("cursor-native").unwrap();
        for different in ["2026-09-07T01:00:00.123456789013Z", "invalid"] {
            record.source.started_at = Some(different.into());
            assert!(
                write_batch(db.store_mut(), &request(&native, &[record.clone()], None)).is_err()
            );
            assert_eq!(db.store().session("cursor-native").unwrap(), before);
            assert_eq!(db.store().counts().unwrap().records, 1);
        }
    }
}

#[test]
fn writer_first_seen_is_earliest_observation_regardless_of_arrival_order() {
    for order in [[200, 100], [100, 200]] {
        let mut db = TempDb::empty().unwrap();
        let native = context(SessionSource::Transcript);
        let records = [poor("observed")];
        for observed_at in order {
            let mut batch = request(&native, &records, None);
            batch.observed_at = observed_at;
            write_batch(db.store_mut(), &batch).unwrap();
        }
        let before = db.store().records("cursor-native").unwrap();
        assert_eq!(before[0].identity.first_seen_at, Some(100));
        assert!(!before[0].has_conflict);
        let sql = rusqlite::Connection::open(db.path()).unwrap();
        assert_eq!(
            sql.query_row(
                "SELECT first_seen_at,last_seen_at FROM session_sources",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            )
            .unwrap(),
            (100, 200)
        );
        sql.execute_batch("CREATE TRIGGER injected BEFORE INSERT ON source_cursors BEGIN SELECT RAISE(ABORT, 'synthetic cursor failure'); END;").unwrap();
        let cursor = SourceCursor {
            source: SessionSource::Transcript,
            cursor_key: "synthetic".into(),
            position: 1,
            updated_at: 50,
        };
        let mut batch = request(&native, &records, None);
        batch.observed_at = 50;
        batch.cursor = Some(&cursor);
        assert!(write_batch(db.store_mut(), &batch).is_err());
        assert_eq!(db.store().records("cursor-native").unwrap(), before);
        sql.execute_batch("DROP TRIGGER injected").unwrap();
        let outcome = write_batch(db.store_mut(), &batch).unwrap();
        assert_eq!((outcome.records_new, outcome.records_enriched), (0, 1));
        assert!(outcome.events[0].invalidate_measurements);
        assert_eq!(
            db.store().records("cursor-native").unwrap()[0]
                .identity
                .first_seen_at,
            Some(50)
        );
    }
}

#[test]
fn writer_identity_unknowns_limits_and_native_cursor_progress_are_explicit() {
    let mut db = TempDb::empty().unwrap();
    let mut context = context(SessionSource::ReadersCli);
    context.conversation_id = None;
    let rows = [poor("row")];
    let cursor = SourceCursor {
        source: SessionSource::ReadersCli,
        cursor_key: "generation-a".into(),
        position: 1,
        updated_at: 100,
    };
    let mut batch = request(&context, &rows, None);
    batch.cursor = Some(&cursor);
    let output = write_batch(db.store_mut(), &batch).unwrap();
    assert_eq!(output.conversation_id, "cursor-native");
    assert_eq!(output.ack_through, None);
    assert_eq!(output.events[0].backfill_position, Some(1));
    assert_eq!(
        Store::open(db.path())
            .unwrap()
            .source_cursor(cursor.source, &cursor.cursor_key)
            .unwrap(),
        Some(cursor)
    );
    let mut bad = rows[0].clone();
    bad.native.session_id = Some("another".into());
    assert!(write_batch(db.store_mut(), &request(&context, &[bad], None)).is_err());
    let unknown = SourceContext {
        conversation_id: Some("explicit-id".into()),
        source_platform: Some("future-host".into()),
        source_surface: Some("new-surface".into()),
        source: Some(SessionSource::ReadersCli),
        ..Default::default()
    };
    write_batch(db.store_mut(), &request(&unknown, &[poor("other")], None)).unwrap();
    let stored = db.store().session("explicit-id").unwrap().unwrap();
    assert_eq!(stored.meta.source_platform.as_deref(), Some("future-host"));
    assert_eq!(stored.meta.surface.as_deref(), Some("new-surface"));
    assert_eq!(stored.meta.native_session_id, None);
    assert!(
        write_batch(
            db.store_mut(),
            &request(&unknown, &vec![poor("limit"); 2001], None)
        )
        .is_err()
    );
}

#[test]
fn capture_receipts_concurrent_exact_retry_has_one_durable_receipt() {
    let db = TempDb::empty().unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let path = db.path().to_owned();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = Store::open(path).unwrap();
                let context = context(SessionSource::Plugin);
                let receipt = receipt("concurrent");
                let rows = [rich("same")];
                barrier.wait();
                write_batch(&mut store, &request(&context, &rows, Some(&receipt))).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .map(|result| result.records_new)
            .sum::<usize>(),
        1
    );
    assert!(
        results
            .iter()
            .all(|result| result.ack_through.as_deref() == Some("same"))
    );
    assert_eq!(
        db.store().capture_receipts("cursor-native").unwrap().len(),
        1
    );
    assert_eq!(db.store().counts().unwrap().records, 1);
}

#[test]
fn capture_receipts_empty_rejected_and_ambiguous_inputs_never_gain_coverage() {
    let mut db = TempDb::empty().unwrap();
    let plugin = context(SessionSource::Plugin);
    let parent = receipt("empty");
    let output = write_batch(db.store_mut(), &request(&plugin, &[], Some(&parent))).unwrap();
    assert_eq!(output.ack_through, None);
    assert!(
        db.store()
            .capture_receipts("cursor-native")
            .unwrap()
            .is_empty()
    );
    let native = context(SessionSource::ReadersCli);
    write_batch(db.store_mut(), &request(&native, &[rich("same")], None)).unwrap();
    let mut rejected = rich("same");
    rejected.canonical.record_type = xt_store::model::RecordType::User;
    let parent = receipt("rejected");
    let result = write_batch(
        db.store_mut(),
        &request(&plugin, &[rejected.clone()], Some(&parent)),
    )
    .unwrap();
    assert_eq!(result.records_dropped, 1);
    assert_eq!(result.ack_through, None);
    assert!(db.store().capture_coverage("rejected").unwrap().is_empty());
    let parent = receipt("mixed");
    assert!(
        write_batch(
            db.store_mut(),
            &request(&plugin, &[rich("same"), rejected], Some(&parent))
        )
        .is_err()
    );
    let mut missing = poor("missing");
    missing.canonical.uuid = None;
    missing.native.session_id = Some("wrong-but-unidentified".into());
    missing.canonical.timestamp = Some("invalid but unidentified".into());
    let parent = receipt("missing");
    let output = write_batch(db.store_mut(), &request(&plugin, &[missing], Some(&parent))).unwrap();
    assert_eq!(output.records_dropped, 1);
    assert_eq!(output.ack_through, None);
    assert!(
        db.store()
            .capture_receipts("cursor-native")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn writer_shared_f1_and_bounded_native_chunks_replay_without_new_records() {
    let fixture = xt_fixtures::Fixture::load(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F1"),
    )
    .unwrap();
    let session = &fixture.sessions()[0];
    let fixture_context = SourceContext {
        conversation_id: Some(session.metadata.session_id.clone()),
        source: Some(SessionSource::Fixture),
        ..Default::default()
    };
    let rows = session
        .records
        .iter()
        .map(|row| parsed(serde_json::to_value(row).unwrap()))
        .collect::<Vec<_>>();
    let mut db = TempDb::empty().unwrap();
    let mut batch = request(&fixture_context, &rows, None);
    batch.declared_host = Some(session.metadata.host);
    assert_eq!(write_batch(db.store_mut(), &batch).unwrap().records_new, 25);
    assert_eq!(write_batch(db.store_mut(), &batch).unwrap().records_new, 0);
    let saved = db
        .store()
        .session(&session.metadata.session_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.meta.host, session.metadata.host);
    assert_eq!(saved.meta.source_platform, None);
    assert!(
        db.store()
            .records(&session.metadata.session_id)
            .unwrap()
            .iter()
            .all(|row| row.classification.is_human.is_none())
    );
    let context = context(SessionSource::ReadersCli);
    let rows = (0..2501)
        .map(|index| poor(&format!("chunk-{index}")))
        .collect::<Vec<_>>();
    let mut position = 0;
    for chunk in rows.chunks(2000) {
        position += chunk.len() as i64;
        let cursor = SourceCursor {
            source: SessionSource::ReadersCli,
            cursor_key: "generation-1".into(),
            position,
            updated_at: 100,
        };
        let mut request = request(&context, chunk, None);
        request.cursor = Some(&cursor);
        let output = write_batch(db.store_mut(), &request).unwrap();
        assert_eq!(output.records_new, chunk.len());
        assert_eq!(output.events[0].backfill_position, Some(position));
    }
    assert_eq!(
        db.store()
            .source_cursor(SessionSource::ReadersCli, "generation-1")
            .unwrap()
            .unwrap()
            .position,
        2501
    );
    assert_eq!(db.store().records("cursor-native").unwrap().len(), 2501);
}

#[test]
fn writer_hygiene_is_derived_before_discard_with_unknowns_preserved() {
    let mut db = TempDb::empty().unwrap();
    let context = context(SessionSource::ReadersCli);
    let cases = [
        ("human", "Hello", true),
        ("command", "<command-name>/test</command-name>", false),
        ("interrupt", "[Request interrupted by user]", false),
        (
            "reminder",
            "<system-reminder>synthetic</system-reminder>",
            false,
        ),
        ("empty", "", false),
    ];
    let rows = cases
        .iter()
        .map(|(uuid, text, _)| {
            parsed(json!({"uuid":uuid,"type":"user","message":{"role":"user","content":text}}))
        })
        .collect::<Vec<_>>();
    write_batch(db.store_mut(), &request(&context, &rows, None)).unwrap();
    for row in db.store().records("cursor-native").unwrap() {
        assert_eq!(row.classification.is_human, None);
        assert_eq!(row.classification.is_command, Some(row.uuid == "command"));
        assert_eq!(
            row.classification.is_interrupted,
            Some(row.uuid == "interrupt")
        );
        assert_eq!(
            row.classification.is_system_reminder,
            Some(row.uuid == "reminder")
        );
        assert!(row.content_json.is_none());
    }
    let roles = [
        parsed(
            json!({"uuid":"role-mismatch","type":"user","message":{"role":"assistant","content":"Text"}}),
        ),
        parsed(json!({"uuid":"role-absent","type":"user","message":{"content":"Text"}})),
    ];
    write_batch(db.store_mut(), &request(&context, &roles, None)).unwrap();
    for row in db
        .store()
        .records("cursor-native")
        .unwrap()
        .into_iter()
        .filter(|row| row.uuid.starts_with("role-"))
    {
        assert_eq!(row.classification.is_human, None);
        assert_eq!(
            row.role.as_deref(),
            if row.uuid == "role-mismatch" {
                Some("assistant")
            } else {
                None
            }
        );
    }
    write_batch(db.store_mut(), &request(&context, &[poor("unknown")], None)).unwrap();
    assert_eq!(
        db.store()
            .records("cursor-native")
            .unwrap()
            .iter()
            .find(|row| row.uuid == "unknown")
            .unwrap()
            .classification
            .is_human,
        None
    );
}

#[test]
fn writer_record_derived_session_conflict_invalidates_without_measurement_enrichment() {
    let mut db = TempDb::empty().unwrap();
    let context = context(SessionSource::ReadersCli);
    let mut row = poor("session-conflict");
    row.canonical.cwd = Some("/synthetic/one".into());
    write_batch(db.store_mut(), &request(&context, &[row.clone()], None)).unwrap();
    row.canonical.cwd = Some("/synthetic/two".into());
    let outcome = write_batch(db.store_mut(), &request(&context, &[row], None)).unwrap();
    assert_eq!((outcome.records_new, outcome.records_enriched), (0, 0));
    let session = db.store().session("cursor-native").unwrap().unwrap();
    assert_eq!(session.meta.cwd.as_deref(), Some("/synthetic/one"));
    assert!(session.has_conflict);
    assert!(outcome.events[0].invalidate_measurements);
    assert!(outcome.events[0].invalidate_cost);
}

#[test]
fn writer_foreign_uuid_invalidates_original_owner_only_after_commit() {
    let mut db = TempDb::empty().unwrap();
    let owner = context(SessionSource::ReadersCli);
    let plugin = context(SessionSource::Plugin);
    let captured = receipt("owner-receipt");
    write_batch(db.store_mut(), &request(&owner, &[rich("owned")], None)).unwrap();
    write_batch(
        db.store_mut(),
        &request(&plugin, &[rich("owned")], Some(&captured)),
    )
    .unwrap();
    let coverage = db.store().capture_coverage("owner-receipt").unwrap();
    let previous = db.store().records("cursor-native").unwrap();
    let mut cached = matches_current(&coverage[0], &previous[0]).unwrap();
    assert!(cached);
    let foreign = SourceContext {
        conversation_id: Some("other-session".into()),
        source: Some(SessionSource::ReadersCli),
        source_surface: Some("ide".into()),
        ..Default::default()
    };
    let sql = rusqlite::Connection::open(db.path()).unwrap();
    sql.execute_batch("CREATE TRIGGER injected BEFORE INSERT ON source_cursors BEGIN SELECT RAISE(ABORT, 'synthetic cursor failure'); END;").unwrap();
    let cursor = SourceCursor {
        source: SessionSource::ReadersCli,
        cursor_key: "foreign".into(),
        position: 1,
        updated_at: 100,
    };
    let rows = [poor("owned"), poor("owned")];
    let mut batch = request(&foreign, &rows, None);
    batch.cursor = Some(&cursor);
    assert!(write_batch(db.store_mut(), &batch).is_err());
    assert_eq!(db.store().records("cursor-native").unwrap(), previous);
    assert!(db.store().session("other-session").unwrap().is_none());
    sql.execute_batch("DROP TRIGGER injected").unwrap();
    let output = write_batch(db.store_mut(), &batch).unwrap();
    assert_eq!((output.records_new, output.records_dropped), (0, 1));
    let owner_events = output
        .events
        .iter()
        .filter(|event| event.conversation_id == "cursor-native")
        .collect::<Vec<_>>();
    assert_eq!(owner_events.len(), 1);
    let event = owner_events[0];
    assert_eq!(event.surface.as_deref(), Some("cli"));
    assert_eq!(event.backfill_position, None);
    assert_eq!((event.records_new, event.records_enriched), (0, 0));
    assert!(event.invalidate_measurements && event.invalidate_cost);
    // A consumer refreshes its independently loaded inputs after the owner event.
    if event.invalidate_measurements {
        cached = matches_current(
            &coverage[0],
            &db.store().records("cursor-native").unwrap()[0],
        )
        .unwrap();
    }
    assert!(!cached);
    assert_eq!(
        db.store().capture_coverage("owner-receipt").unwrap(),
        coverage
    );
    assert!(
        db.store()
            .capture_receipts("other-session")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn capture_receipts_retry_binds_parent_and_complete_unordered_set() {
    for mutation in [
        "time",
        "surface",
        "session",
        "missing",
        "extra",
        "measurement",
    ] {
        let mut db = TempDb::empty().unwrap();
        let plugin = context(SessionSource::Plugin);
        let original = receipt("stable-parent");
        let rows = [poor("first"), rich("second")];
        write_batch(db.store_mut(), &request(&plugin, &rows, Some(&original))).unwrap();
        let retry = write_batch(
            db.store_mut(),
            &request(
                &plugin,
                &[rows[1].clone(), rows[0].clone()],
                Some(&original),
            ),
        )
        .unwrap();
        assert_eq!(retry.ack_through.as_deref(), Some("first"));
        assert_eq!((retry.records_new, retry.records_enriched), (0, 0));
        let before = db.store().records("cursor-native").unwrap();
        let evidence = db.store().capture_coverage("stable-parent").unwrap();
        let mut parent = original.clone();
        let mut candidate = rows.to_vec();
        let mut candidate_context = plugin.clone();
        match mutation {
            "time" => parent.received_at += 1,
            "surface" => {
                parent.surface = Some("ide".into());
                candidate_context.source_surface = parent.surface.clone();
            }
            "session" => {
                parent.session_id = "cursor-elsewhere".into();
                candidate_context.conversation_id = Some(parent.session_id.clone());
                candidate_context.native_session_id = Some("elsewhere".into());
                candidate = vec![poor("new-owner")];
            }
            "missing" => {
                candidate.pop();
            }
            "extra" => candidate.push(poor("new")),
            "measurement" => candidate[0] = rich("first"),
            _ => unreachable!(),
        }
        assert!(
            write_batch(
                db.store_mut(),
                &request(&candidate_context, &candidate, Some(&parent))
            )
            .is_err(),
            "{mutation}"
        );
        assert_eq!(
            db.store().records("cursor-native").unwrap(),
            before,
            "{mutation}"
        );
        assert_eq!(
            db.store().capture_receipts("cursor-native").unwrap(),
            [original]
        );
        assert_eq!(
            db.store().capture_coverage("stable-parent").unwrap(),
            evidence
        );
        assert!(db.store().session("cursor-elsewhere").unwrap().is_none());
    }
}

#[test]
fn capture_receipts_empty_retry_cannot_commit_metadata_conflicts_or_cursor() {
    for variant in ["empty", "missing", "rejected"] {
        let mut db = TempDb::empty().unwrap();
        let context = context(SessionSource::Plugin);
        let receipt = receipt("sealed");
        let rows = [poor("saved")];
        let cursor = SourceCursor {
            source: SessionSource::Plugin,
            cursor_key: "delivery".into(),
            position: 1,
            updated_at: 100,
        };
        let mut initial = request(&context, &rows, Some(&receipt));
        initial.cursor = Some(&cursor);
        write_batch(db.store_mut(), &initial).unwrap();
        let before_session = db.store().session("cursor-native").unwrap();
        let before_rows = db.store().records("cursor-native").unwrap();
        let before_sources = db.store().session_sources("cursor-native").unwrap();
        let before_coverage = db.store().capture_coverage("sealed").unwrap();
        let rows = match variant {
            "empty" => vec![],
            "missing" => {
                let mut missing = poor("missing");
                missing.canonical.uuid = None;
                vec![missing]
            }
            "rejected" => {
                let mut rejected = poor("saved");
                rejected.canonical.record_type = xt_store::model::RecordType::User;
                vec![rejected]
            }
            _ => unreachable!(),
        };
        let next = SourceCursor {
            position: 2,
            updated_at: 200,
            ..cursor.clone()
        };
        let mut retry = request(&context, &rows, Some(&receipt));
        retry.cursor = Some(&next);
        retry.keep_content = true;
        retry.title = Some("Must roll back");
        retry.observed_at = 200;
        assert!(write_batch(db.store_mut(), &retry).is_err(), "{variant}");
        assert_eq!(db.store().session("cursor-native").unwrap(), before_session);
        assert_eq!(db.store().records("cursor-native").unwrap(), before_rows);
        assert_eq!(
            db.store().session_sources("cursor-native").unwrap(),
            before_sources
        );
        assert_eq!(
            db.store().capture_coverage("sealed").unwrap(),
            before_coverage
        );
        assert_eq!(
            db.store()
                .source_cursor(cursor.source, &cursor.cursor_key)
                .unwrap(),
            Some(cursor)
        );
    }
}

#[test]
fn writer_retained_content_conflict_invalidates_cached_receipt_without_new_measurements() {
    for tool_input in [false, true] {
        let mut db = TempDb::empty().unwrap();
        db.store_mut()
            .set_retention_mode(xt_store::retention::RetentionMode::FullContent)
            .unwrap();
        let plugin = context(SessionSource::Plugin);
        let native = context(SessionSource::ReadersCli);
        let receipt = receipt("retained");
        let rows = [rich("retained")];
        let mut original = request(&plugin, &rows, Some(&receipt));
        original.keep_content = true;
        write_batch(db.store_mut(), &original).unwrap();
        let evidence = db.store().capture_coverage("retained").unwrap();
        let before = db.store().records("cursor-native").unwrap();
        assert!(matches_current(&evidence[0], &before[0]).unwrap());
        let mut altered = rows[0].clone();
        if tool_input {
            altered.canonical.message.content.as_mut().unwrap()[1]["input"] =
                json!({"different":"synthetic value"});
        } else {
            altered.canonical.message.content.as_mut().unwrap()[0]["text"] =
                json!("Other text xxx");
        }
        assert_eq!(coverage("cursor-native", &altered).unwrap(), evidence[0]);
        let rows = [altered];
        let mut replay = request(&native, &rows, None);
        replay.keep_content = true;
        let output = write_batch(db.store_mut(), &replay).unwrap();
        assert_eq!((output.records_new, output.records_enriched), (0, 0));
        assert!(output.events[0].invalidate_measurements);
        assert!(output.events[0].invalidate_cost);
        let after = db.store().records("cursor-native").unwrap();
        assert!(after[0].has_conflict);
        assert_eq!(after[0].content_json, before[0].content_json);
        assert_eq!(after[0].tool_uses, before[0].tool_uses);
        assert!(!matches_current(&evidence[0], &after[0]).unwrap());
        assert_eq!(db.store().capture_coverage("retained").unwrap(), evidence);
    }
}

#[test]
fn writer_event_surface_comes_from_the_committed_session() {
    let mut db = TempDb::empty().unwrap();
    let known = context(SessionSource::ReadersCli);
    write_batch(db.store_mut(), &request(&known, &[poor("known")], None)).unwrap();
    let mut sparse = known.clone();
    sparse.source_surface = None;
    let output = write_batch(db.store_mut(), &request(&sparse, &[poor("known")], None)).unwrap();
    let stored = db.store().session("cursor-native").unwrap().unwrap();
    assert_eq!(stored.meta.surface.as_deref(), Some("cli"));
    assert_eq!(output.events[0].surface, stored.meta.surface);
    let mut conflicting = known.clone();
    conflicting.source_surface = Some("ide".into());
    let output = write_batch(
        db.store_mut(),
        &request(&conflicting, &[poor("known")], None),
    )
    .unwrap();
    let stored = db.store().session("cursor-native").unwrap().unwrap();
    assert!(stored.has_conflict);
    assert_eq!(stored.meta.surface.as_deref(), Some("cli"));
    assert_eq!(output.events[0].surface, stored.meta.surface);
}
