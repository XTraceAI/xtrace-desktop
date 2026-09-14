use xt_store::{SessionSource, Store, batch::SourceCursor};

#[test]
fn discovery_ownership_must_match_the_batch_before_any_write() {
    use xt_store::{Host, SessionMeta, batch::IngestBatch, ingest::DiscoveredSession};
    let mut session = SessionMeta::new("claude-session", "claude", SessionSource::Transcript);
    session.native_session_id = Some("native-session".into());
    session.surface = Some("cli".into());
    session.started_at_ms = Some(1000);
    for field in ["host", "native", "conversation", "surface", "start"] {
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
            "surface" => discovery.surface = Some("sdk".into()),
            "start" => discovery.started_at_ms = Some(2000),
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

#[test]
fn native_locators_never_claim_resume_offsets_or_need_scan_ordering() {
    let temp = tempfile::TempDir::new().unwrap();
    let path = temp.path().join("index.sqlite");
    let mut a = Store::open(&path).unwrap();
    let mut b = Store::open(&path).unwrap();
    let locator = |time| SourceCursor {
        source: SessionSource::Transcript,
        cursor_key: "source".into(),
        position: 0,
        updated_at: time,
    };
    a.record_native_source_locator(&locator(1), false).unwrap();
    assert!(
        a.source_cursor(SessionSource::Transcript, "source")
            .unwrap()
            .is_none()
    );
    // Migrate any older development build's saved offset to safe full replay.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute(
            "INSERT INTO source_cursors(source,cursor_key,position,updated_at) VALUES (?1, 'source', 1000, 50)",
            [SessionSource::Transcript],
        )
        .unwrap();
    for (time, create) in [(50, true), (20, false), (50, false), (60, true)] {
        b.record_native_source_locator(&locator(time), create)
            .unwrap();
        assert_eq!(
            a.source_cursor(SessionSource::Transcript, "source")
                .unwrap()
                .unwrap()
                .position,
            0
        );
    }
    assert_eq!(
        a.source_cursor(SessionSource::Transcript, "source")
            .unwrap()
            .unwrap()
            .updated_at,
        60
    );
}
