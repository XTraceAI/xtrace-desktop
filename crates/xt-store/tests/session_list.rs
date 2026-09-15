use xt_store::{SessionMeta, SessionSource, Store};

#[test]
fn session_pages_are_bounded_stable_filtered_and_content_free() {
    let mut store = Store::open_in_memory().unwrap();
    for i in 0..123 {
        let mut session = SessionMeta::new(
            format!("session-{i:03}"),
            if i % 2 == 0 { "claude" } else { "codex" },
            SessionSource::Transcript,
        );
        session.started_at_ms = if i < 120 { Some(1000) } else { None };
        session.cwd = Some("/repo/project".into());
        session.git_branch = Some(if i == 4 { "feature%literal" } else { "main" }.into());
        store.upsert_session(&session, false).unwrap();
    }
    let mut cursor = None;
    let mut ids = std::collections::BTreeSet::new();
    loop {
        let mut rows = store.sessions_page("", None, cursor.as_ref()).unwrap();
        let more = rows.len() > 50;
        rows.truncate(50);
        for row in &rows {
            assert!(ids.insert(row.id.clone()));
            assert_eq!(row.record_count, 0);
            assert_eq!(row.model, None);
        }
        if !more {
            break;
        }
        cursor = Some(rows.last().unwrap().cursor.clone());
    }
    assert_eq!(ids.len(), 123);
    assert_eq!(
        store.sessions_page("feature%", None, None).unwrap()[0].id,
        "session-004"
    );
    assert!(
        store
            .sessions_page("", Some("codex"), None)
            .unwrap()
            .iter()
            .all(|r| r.host == "codex")
    );
    assert!(
        store
            .sessions_page("missing", None, None)
            .unwrap()
            .is_empty()
    );
    assert!(store.sessions_page("", Some("bogus"), None).is_err());
    assert!(store.sessions_page(&"x".repeat(257), None, None).is_err());
}
