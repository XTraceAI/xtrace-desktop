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

#[test]
fn stale_metadata_ranges_do_not_control_display_or_pagination() {
    let temp = tempfile::TempDir::new().unwrap();
    let path = temp.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    let exact_first = "2026-01-02T19:00:00.123456789011-05:00";
    for (id, rows) in [
        (
            "newer-work",
            vec![
                ("meta", "2026-01-01T00:00:00Z", true),
                ("a", "2026-01-03T00:00:00.123456789012Z", false),
                ("z", exact_first, false),
            ],
        ),
        ("older-work", vec![("b", "2026-01-02T00:00:00Z", false)]),
        ("only-meta", vec![("only", "2099-01-01T00:00:00Z", true)]),
    ] {
        store
            .upsert_session(
                &SessionMeta::new(id, "codex", SessionSource::ReadersCli),
                false,
            )
            .unwrap();
        let records = rows
            .into_iter()
            .map(|(uuid, timestamp, meta)| {
                serde_json::from_value(serde_json::json!({
            "uuid":uuid,"type":"assistant","timestamp":timestamp,"isMeta":meta,
            "message":{"role":"assistant","model":if meta {"inherited"} else {"own"},"content":[]}
        })).unwrap()
            })
            .collect::<Vec<_>>();
        store.upsert_records(id, &records, false).unwrap();
    }
    // Simulate persisted ranges written before metadata was excluded.
    let sql = rusqlite::Connection::open(path).unwrap();
    sql.execute(
        "UPDATE sessions SET first_ts='2026-01-01T00:00:00Z' WHERE session_id='newer-work'",
        [],
    )
    .unwrap();
    sql.execute(
        "UPDATE sessions SET first_ts='2099-01-01T00:00:00Z' WHERE session_id='only-meta'",
        [],
    )
    .unwrap();
    let rows = store.sessions_page("", None, None).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["newer-work", "older-work", "only-meta"]
    );
    assert_eq!(rows[0].first_ts.as_deref(), Some(exact_first));
    assert_eq!(rows[0].record_count, 2);
    assert_eq!(rows[0].model.as_deref(), Some("own"));
    assert_eq!(rows[2].first_ts, None);
    assert_eq!(rows[2].record_count, 0);
    let next = store
        .sessions_page("", None, Some(&rows[0].cursor))
        .unwrap();
    assert_eq!(
        next.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["older-work", "only-meta"]
    );
    assert_eq!(
        sql.query_row(
            "SELECT first_ts FROM sessions WHERE session_id='only-meta'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "2099-01-01T00:00:00Z"
    );
    store
        .upsert_session(
            &SessionMeta::new("copied-work", "claude", SessionSource::Transcript),
            false,
        )
        .unwrap();
    sql.execute("INSERT INTO native_record_copies(session_id,record_uuid) VALUES('copied-work','a'),('copied-work','meta')",[]).unwrap();
    let copied = store.sessions_page("copied-work", None, None).unwrap();
    assert_eq!(copied[0].record_count, 1);
    assert_eq!(copied[0].model.as_deref(), Some("own"));
    assert_eq!(
        copied[0].first_ts.as_deref(),
        Some("2026-01-03T00:00:00.123456789012Z")
    );
}
