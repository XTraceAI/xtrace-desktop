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
    newer.record_completed_source_scan(&cursor(12, 50)).unwrap();
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
