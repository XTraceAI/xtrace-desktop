//! The bounded context the Dashboard's session rows read: the same saved
//! title, native start and recorded pull-request links the Sessions page
//! shows, for exactly the named sessions, on the read-only metrics connection.
use xt_fixtures::TempDb;
use xt_metrics::MetricsDb;
use xt_store::pr_link::{PrConfidence, PrIdentity, PrLinkObservation};
use xt_store::session_list::SessionFilter;
use xt_store::{SessionMeta, SessionSource, Store};

fn session(store: &mut Store, id: &str, started: Option<i64>, title: Option<&str>) {
    let mut session = SessionMeta::new(id, "claude", SessionSource::Fixture);
    session.started_at_ms = started;
    session.title = title.map(str::to_owned);
    session.cwd = Some("/repo/project".into());
    store.upsert_session(&session, title.is_some()).unwrap();
}

fn link(store: &mut Store, id: &str, number: u64, confidence: PrConfidence) {
    store
        .record_pr_link(&PrLinkObservation {
            session_id: id.into(),
            pull_request: PrIdentity::from_parts("example/atlas", number).unwrap(),
            confidence,
            first_seen_at: 1,
            last_seen_at: 1,
        })
        .unwrap();
}

#[test]
fn context_carries_saved_title_native_start_and_recorded_link_counts() {
    let mut db = TempDb::empty().unwrap();
    let store = db.store_mut();
    // Titles are content: only a store that retains content saves one.
    store
        .set_retention_mode(xt_store::retention::RetentionMode::FullContent)
        .unwrap();
    session(store, "mixed", Some(100), Some("Saved   title"));
    session(store, "blank", None, Some("  "));
    session(store, "unlinked", Some(5), None);
    session(store, "sharing", Some(7), None);
    link(store, "mixed", 9, PrConfidence::Exact);
    link(store, "mixed", 3, PrConfidence::Inferred);
    link(store, "mixed", 7, PrConfidence::Inferred);
    link(store, "mixed", 11, PrConfidence::Sha);
    // A replay of the same identity is still one link.
    link(store, "mixed", 9, PrConfidence::Exact);
    link(store, "blank", 4, PrConfidence::Sha);
    // One pull request shared by two sessions counts once for each.
    link(store, "sharing", 9, PrConfidence::Exact);

    let metrics = MetricsDb::open(db.path()).unwrap();
    let rows = metrics
        .session_context(&["unlinked", "mixed", "absent", "blank", "sharing", "mixed"])
        .unwrap();
    let summary: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r.id.as_str(),
                r.title.as_deref(),
                r.started_at_ms,
                r.pr_links,
                r.inferred_pr_links,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            // A blank saved title reads as absent; a missing start stays unknown.
            ("blank", None, None, 1, 0),
            ("mixed", Some("Saved   title"), Some(100), 4, 2),
            ("sharing", None, Some(7), 1, 0),
            // An indexed session with no link is a measured zero.
            ("unlinked", None, Some(5), 0, 0),
        ]
    );
    // An identifier no indexed session owns is absent, never a zero row.
    assert!(rows.iter().all(|r| r.id != "absent"));

    // The counts, title and start agree with the full Sessions projection.
    let listed = metrics
        .sessions_page_filtered(&SessionFilter::default(), None)
        .unwrap();
    for row in &rows {
        let page = listed.iter().find(|r| r.id == row.id).unwrap();
        assert_eq!(row.pr_links, page.pr_links.len() as u64);
        assert_eq!(
            row.inferred_pr_links,
            page.pr_links
                .iter()
                .filter(|l| l.confidence == PrConfidence::Inferred)
                .count() as u64
        );
        assert_eq!(
            (&row.title, row.started_at_ms),
            (&page.title, page.started_at_ms)
        );
    }
}
