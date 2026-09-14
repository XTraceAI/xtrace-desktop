use xt_store::{SessionSource, Store, batch::SourceCursor};

#[test]
fn completed_scan_ordering_preserves_newer_replacements_and_empty_resets() {
    let temp = tempfile::TempDir::new().unwrap();
    let path = temp.path().join("index.sqlite");
    let mut older = Store::open(&path).unwrap();
    let mut newer = Store::open(&path).unwrap();
    let cursor = |position, updated_at| SourceCursor {
        source: SessionSource::Transcript,
        cursor_key: "synthetic-source".into(),
        position,
        updated_at,
    };
    older
        .record_completed_source_scan(&cursor(1000, 10))
        .unwrap();
    newer.record_completed_source_scan(&cursor(20, 30)).unwrap();
    older
        .record_completed_source_scan(&cursor(900, 20))
        .unwrap();
    older
        .reset_existing_source_cursor(SessionSource::Transcript, "synthetic-source", 20)
        .unwrap();
    assert_eq!(
        newer
            .source_cursor(SessionSource::Transcript, "synthetic-source")
            .unwrap(),
        Some(cursor(20, 30))
    );
    newer
        .reset_existing_source_cursor(SessionSource::Transcript, "synthetic-source", 40)
        .unwrap();
    older
        .record_completed_source_scan(&cursor(1000, 30))
        .unwrap();
    assert_eq!(
        older
            .source_cursor(SessionSource::Transcript, "synthetic-source")
            .unwrap(),
        Some(cursor(0, 40))
    );
    // Equal timestamps are ambiguous: retain the smaller safe offset.
    older
        .record_completed_source_scan(&cursor(1000, 40))
        .unwrap();
    assert_eq!(
        older
            .source_cursor(SessionSource::Transcript, "synthetic-source")
            .unwrap(),
        Some(cursor(0, 40))
    );
    newer.record_completed_source_scan(&cursor(12, 50)).unwrap();
    older
        .record_completed_source_scan(&cursor(1000, 50))
        .unwrap();

    assert_eq!(
        older
            .source_cursor(SessionSource::Transcript, "synthetic-source")
            .unwrap(),
        Some(cursor(12, 50))
    );
    newer
        .reset_existing_source_cursor(SessionSource::Transcript, "never-imported", 50)
        .unwrap();
    assert!(
        older
            .source_cursor(SessionSource::Transcript, "never-imported")
            .unwrap()
            .is_none()
    );
}

#[test]
fn equal_time_scans_keep_the_smaller_offset_in_both_commit_orders() {
    for (first, second) in [(1000, 20), (20, 1000)] {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("index.sqlite");
        let mut a = Store::open(&path).unwrap();
        let mut b = Store::open(&path).unwrap();
        let cursor = |position| SourceCursor {
            source: SessionSource::Transcript,
            cursor_key: "same-time".into(),
            position,
            updated_at: 10,
        };
        a.record_completed_source_scan(&cursor(first)).unwrap();
        b.record_completed_source_scan(&cursor(second)).unwrap();
        assert_eq!(
            a.source_cursor(SessionSource::Transcript, "same-time")
                .unwrap(),
            Some(cursor(20))
        );
        a.reset_existing_source_cursor(SessionSource::Transcript, "same-time", 10)
            .unwrap();
        b.record_completed_source_scan(&cursor(1000)).unwrap();
        assert_eq!(
            a.source_cursor(SessionSource::Transcript, "same-time")
                .unwrap(),
            Some(cursor(0))
        );
    }
}

#[test]
fn discovery_ownership_must_match_the_batch_before_any_write() {
    use xt_store::{Host, SessionMeta, batch::IngestBatch, ingest::DiscoveredSession};
    let mut session = SessionMeta::new("claude-session", "claude", SessionSource::Transcript);
    session.native_session_id = Some("native-session".into());
    for field in ["host", "native", "conversation"] {
        let mut store = Store::open_in_memory().unwrap();
        let mut discovery = DiscoveredSession {
            host: Host::Claude,
            native_session_id: "native-session".into(),
            conversation_id: Some("claude-session".into()),
            surface: None,
            started_at_ms: None,
            last_observed_at: 1,
            discovery_complete: true,
        };
        match field {
            "host" => discovery.host = Host::Codex,
            "native" => discovery.native_session_id = "other".into(),
            _ => discovery.conversation_id = Some("other".into()),
        }
        let mut batch = IngestBatch::new(&session, &[], false);
        batch.discovery = Some(&discovery);
        assert!(store.apply_ingest_batch(&batch).is_err());
        assert_eq!(store.counts().unwrap().sessions, 0);
        assert!(
            store
                .discovered_sessions(discovery.host)
                .unwrap()
                .is_empty()
        );
    }
}
