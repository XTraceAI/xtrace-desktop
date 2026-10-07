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
    // Copied work is its owner's: it names no model and sets no start here,
    // though it is still the earliest visible work.
    assert_eq!(copied[0].model, None);
    assert_eq!(copied[0].other_models, 0);
    assert_eq!(copied[0].started_at_ms, None);
    assert_eq!(
        copied[0].first_ts.as_deref(),
        Some("2026-01-03T00:00:00.123456789012Z")
    );
}

/// Claude Code transcripts record no session start, so a Claude session with
/// none stored shows its earliest imported message as its start. A stored
/// start is kept, other hosts never get one invented, and nothing is written
/// back to the stored session.
#[test]
fn claude_sessions_without_a_stored_start_start_at_their_earliest_message() {
    let temp = tempfile::TempDir::new().unwrap();
    let path = temp.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    let earliest_ms = 1_767_312_000_000; // 2026-01-02T00:00:00Z
    for (id, host, started) in [
        ("claude-unstarted", "claude", None),
        ("claude-started", "claude", Some(5)),
        ("codex-unstarted", "codex", None),
        ("claude-empty", "claude", None),
    ] {
        let mut session = SessionMeta::new(id, host, SessionSource::Transcript);
        session.started_at_ms = started;
        store.upsert_session(&session, false).unwrap();
        if id == "claude-empty" {
            continue;
        }
        let records = [
            // A meta record is not a message and never sets the start.
            (format!("{id}-meta"), "2026-01-01T00:00:00Z", true),
            (format!("{id}-late"), "2026-01-03T00:00:00Z", false),
            (format!("{id}-early"), "2026-01-02T00:00:00Z", false),
        ]
        .into_iter()
        .map(|(uuid, timestamp, meta)| {
            serde_json::from_value(serde_json::json!({
                "uuid":uuid,"type":"assistant","timestamp":timestamp,"isMeta":meta,
                "message":{"role":"assistant","model":"m","content":[]}
            }))
            .unwrap()
        })
        .collect::<Vec<_>>();
        store.upsert_records(id, &records, false).unwrap();
    }
    let expected = [
        ("claude-empty", None),
        ("claude-started", Some(5)),
        ("claude-unstarted", Some(earliest_ms)),
        ("codex-unstarted", None),
    ];

    let rows = store.sessions_page("", None, None).unwrap();
    let mut listed: Vec<_> = rows
        .iter()
        .map(|r| (r.id.as_str(), r.started_at_ms))
        .collect();
    listed.sort();
    assert_eq!(listed, expected);
    // The shown start is the same instant the row's first work names.
    let unstarted = rows.iter().find(|r| r.id == "claude-unstarted").unwrap();
    assert_eq!(unstarted.first_ts.as_deref(), Some("2026-01-02T00:00:00Z"));

    let sql = rusqlite::Connection::open(&path).unwrap();
    xt_store::timestamp::register_sqlite(&sql).unwrap();
    for (id, start) in expected {
        let one = xt_store::session_list::exact(&sql, id).unwrap().unwrap();
        assert_eq!(one.started_at_ms, start, "{id}");
    }
    let context = xt_store::session_list::context(
        &sql,
        &[
            "claude-unstarted",
            "claude-started",
            "codex-unstarted",
            "claude-empty",
        ],
    )
    .unwrap();
    assert_eq!(
        context
            .iter()
            .map(|r| (r.id.as_str(), r.started_at_ms))
            .collect::<Vec<_>>(),
        expected
    );

    // The stored start is untouched: only the read shows the earliest message.
    let stored: Vec<(String, Option<i64>)> = sql
        .prepare("SELECT session_id,started_at_ms FROM sessions ORDER BY session_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        stored,
        [
            ("claude-empty".to_owned(), None),
            ("claude-started".to_owned(), Some(5)),
            ("claude-unstarted".to_owned(), None),
            ("codex-unstarted".to_owned(), None),
        ]
    );
}

mod design_projection {
    use xt_store::pr_link::{PrConfidence, PrIdentity, PrLinkObservation};
    use xt_store::session_list::{SessionCursor, SessionFilter, SessionSummary};
    use xt_store::{SessionMeta, SessionSource, Store};

    fn session(store: &mut Store, id: &str, host: &str, started: Option<i64>, title: Option<&str>) {
        let mut session = SessionMeta::new(id, host, SessionSource::Fixture);
        session.started_at_ms = started;
        session.title = title.map(str::to_owned);
        session.cwd = Some("/repo/project".into());
        // A title is only acquired while content is retained; this reads
        // whatever was saved and never derives one.
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

    /// Every page of a filter, following the cursor, as the app does.
    fn all(store: &Store, filter: SessionFilter<'_>) -> Vec<SessionSummary> {
        let mut out = Vec::new();
        let mut cursor: Option<SessionCursor> = None;
        loop {
            let mut rows = store
                .sessions_page_filtered(&filter, cursor.as_ref())
                .unwrap();
            let more = rows.len() > 50;
            rows.truncate(50);
            cursor = rows.last().map(|row| row.cursor.clone());
            out.extend(rows);
            if !more {
                return out;
            }
        }
    }

    /// Sixty newer unlinked sessions fill more than the first page; the two
    /// linked ones are older. A filter applied after LIMIT would return none.
    fn crowded() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        // Titles are content: only a store that retains content saves one.
        store
            .set_retention_mode(xt_store::retention::RetentionMode::FullContent)
            .unwrap();
        for index in 0..60 {
            let host = ["claude", "codex", "cursor"][index % 3];
            session(
                &mut store,
                &format!("newer-{index:02}"),
                host,
                Some(10_000 + index as i64),
                None,
            );
        }
        session(
            &mut store,
            "linked-old",
            "codex",
            Some(100),
            Some("Saved   title"),
        );
        session(&mut store, "linked-untimed", "claude", None, Some("  "));
        session(&mut store, "other-host", "other", Some(50), None);
        link(&mut store, "linked-old", 7, PrConfidence::Inferred);
        link(&mut store, "linked-old", 9, PrConfidence::Exact);
        link(&mut store, "linked-old", 3, PrConfidence::Inferred);
        link(&mut store, "linked-untimed", 7, PrConfidence::Sha);
        store
    }

    #[test]
    fn with_prs_is_applied_before_the_page_bound() {
        let store = crowded();
        let first = store
            .sessions_page_filtered(
                &SessionFilter {
                    with_prs: true,
                    ..Default::default()
                },
                None,
            )
            .unwrap();
        assert_eq!(
            first.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            // A native start sorts before an unknown one, which falls back to
            // first recorded work and then to the minimum.
            ["linked-old", "linked-untimed"]
        );
        // Unfiltered, both are beyond the first page.
        let unfiltered = store
            .sessions_page_filtered(&SessionFilter::default(), None)
            .unwrap();
        assert!(unfiltered[..50].iter().all(|r| r.pr_links.is_empty()));
        assert_eq!(all(&store, SessionFilter::default()).len(), 63);
    }

    #[test]
    fn links_carry_confidence_strongest_first_and_are_never_merged_across_sessions() {
        let store = crowded();
        let rows = all(
            &store,
            SessionFilter {
                with_prs: true,
                ..Default::default()
            },
        );
        let old = &rows[0];
        assert_eq!(
            old.pr_links
                .iter()
                .map(|l| (l.pull_request.number(), l.confidence))
                .collect::<Vec<_>>(),
            [
                (9, PrConfidence::Exact),
                (3, PrConfidence::Inferred),
                (7, PrConfidence::Inferred)
            ]
        );
        assert!(old.pr_links.iter().all(|l| l.title.is_none()));
        assert_eq!(
            old.pr_links[0].pull_request.url(),
            "https://github.com/example/atlas/pull/9"
        );
        assert_eq!(rows[1].pr_links.len(), 1);
        assert_eq!(rows[1].pr_links[0].confidence, PrConfidence::Sha);
    }

    #[test]
    fn saved_titles_and_native_starts_are_exposed_never_invented() {
        let store = crowded();
        let rows = all(&store, SessionFilter::default());
        let find = |id: &str| rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(find("linked-old").title.as_deref(), Some("Saved   title"));
        assert_eq!(find("linked-old").started_at_ms, Some(100));
        // A blank saved title reads as absent, and a missing start stays unknown.
        assert_eq!(find("linked-untimed").title, None);
        assert_eq!(find("linked-untimed").started_at_ms, None);
        assert_eq!(find("newer-00").title, None);
        // The saved title is searchable, before paging, case-insensitively.
        let found = all(
            &store,
            SessionFilter {
                search: "SAVED",
                ..Default::default()
            },
        );
        assert_eq!(
            found.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["linked-old"]
        );
    }

    #[test]
    fn a_host_set_filters_before_paging_and_rejects_what_the_store_does_not_know() {
        let store = crowded();
        let hosts = ["codex", "cursor", "codex"];
        let rows = all(
            &store,
            SessionFilter {
                hosts: Some(&hosts),
                ..Default::default()
            },
        );
        assert_eq!(rows.len(), 41);
        assert!(rows.iter().all(|r| r.host == "codex" || r.host == "cursor"));
        assert!(rows.iter().any(|r| r.id == "linked-old"));
        // The legacy single-host page is the same filter with one member.
        let single = store.sessions_page("", Some("other"), None).unwrap();
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].id, "other-host");
        // Combined filters intersect.
        let codex = ["codex"];
        let both = all(
            &store,
            SessionFilter {
                hosts: Some(&codex),
                with_prs: true,
                ..Default::default()
            },
        );
        assert_eq!(both.len(), 1);
        for invalid in [&[][..], &["bogus"][..], &["claude", "Claude"][..]] {
            assert!(
                store
                    .sessions_page_filtered(
                        &SessionFilter {
                            hosts: Some(invalid),
                            ..Default::default()
                        },
                        None
                    )
                    .is_err(),
                "{invalid:?}"
            );
        }
    }
}

mod exact_pull_request {
    use std::collections::BTreeSet;
    use xt_store::pr_link::{PrConfidence, PrIdentity, PrLinkObservation};
    use xt_store::session_list::{PrMembership, SessionCursor, SessionFilter, SessionSummary};
    use xt_store::{SessionMeta, SessionSource, Store};

    fn link(store: &mut Store, id: &str, repository: &str, number: u64, confidence: PrConfidence) {
        store
            .record_pr_link(&PrLinkObservation {
                session_id: id.into(),
                pull_request: PrIdentity::from_parts(repository, number).unwrap(),
                confidence,
                first_seen_at: 1,
                last_seen_at: 1,
            })
            .unwrap();
    }

    /// Every page of a filter, following the cursor, with each page's size.
    fn pages(store: &Store, filter: SessionFilter<'_>) -> (Vec<SessionSummary>, Vec<usize>) {
        let (mut out, mut sizes) = (Vec::new(), Vec::new());
        let mut cursor: Option<SessionCursor> = None;
        loop {
            let mut rows = store
                .sessions_page_filtered(&filter, cursor.as_ref())
                .unwrap();
            let more = rows.len() > 50;
            rows.truncate(50);
            sizes.push(rows.len());
            cursor = rows.last().map(|row| row.cursor.clone());
            out.extend(rows);
            if !more {
                return (out, sizes);
            }
        }
    }

    /// 112 sessions linked to `example/atlas#7` (a third only inferred),
    /// interleaved in sort order with 128 that are not: linked to #8, to the
    /// same number in another repository, or to nothing. Pairs share a native
    /// start, and some have none, so ties and the fallback order are crossed.
    /// Returns the store and the members with their link confidence.
    fn interspersed() -> (Store, Vec<(String, PrConfidence)>) {
        let mut store = Store::open_in_memory().unwrap();
        let mut members = Vec::new();
        for index in 0..240_u32 {
            let id = format!("s-{index:03}");
            let host = if index % 2 == 1 { "codex" } else { "claude" };
            let mut session = SessionMeta::new(&id, host, SessionSource::Fixture);
            session.started_at_ms =
                (!index.is_multiple_of(25)).then(|| 1_000 + i64::from(index / 2));
            store.upsert_session(&session, false).unwrap();
            // Odd and even indexes share a start; members fall on both.
            if index % 15 < 7 {
                let confidence = match index % 3 {
                    0 => PrConfidence::Inferred,
                    1 => PrConfidence::Exact,
                    _ => PrConfidence::Sha,
                };
                link(&mut store, &id, "example/atlas", 7, confidence);
                members.push((id.clone(), confidence));
                // Another pull request's link never duplicates a member.
                if index.is_multiple_of(5) {
                    link(&mut store, &id, "example/atlas", 8, PrConfidence::Exact);
                    link(&mut store, &id, "other/atlas", 7, PrConfidence::Exact);
                }
            } else if index % 15 < 10 {
                link(&mut store, &id, "example/atlas", 8, PrConfidence::Exact);
            } else if index % 15 < 13 {
                link(&mut store, &id, "other/atlas", 7, PrConfidence::Exact);
            }
        }
        assert_eq!(members.len(), 112);
        (store, members)
    }

    fn ids(rows: &[SessionSummary]) -> Vec<&str> {
        rows.iter().map(|row| row.id.as_str()).collect()
    }

    #[test]
    fn membership_is_filtered_before_the_page_bound_and_pages_are_complete() {
        let (store, members) = interspersed();
        let identity = PrIdentity::from_parts("example/atlas", 7).unwrap();
        let unfiltered = pages(&store, SessionFilter::default()).0;
        assert_eq!(unfiltered.len(), 240);
        for confirmed_only in [false, true] {
            let (rows, sizes) = pages(
                &store,
                SessionFilter {
                    pull_request: Some(PrMembership {
                        identity: &identity,
                        confirmed_only,
                    }),
                    ..Default::default()
                },
            );
            let expected: BTreeSet<&str> = members
                .iter()
                .filter(|(_, confidence)| !confirmed_only || *confidence != PrConfidence::Inferred)
                .map(|(id, _)| id.as_str())
                .collect();
            // Every member once: a filter applied after LIMIT would thin the
            // first page, and a cursor over unfiltered rows would skip some.
            let listed: BTreeSet<&str> = ids(&rows).into_iter().collect();
            assert_eq!(listed.len(), rows.len(), "no duplicates");
            assert_eq!(listed, expected, "confirmed_only={confirmed_only}");
            assert_eq!(sizes[0], 50);
            assert_eq!(sizes.iter().sum::<usize>(), expected.len());
            // The unfiltered order, restricted to the members.
            let order: Vec<&str> = ids(&unfiltered)
                .into_iter()
                .filter(|id| expected.contains(id))
                .collect();
            assert_eq!(ids(&rows), order);
            // A member's row still carries every stored link.
            let shared = rows.iter().find(|row| row.id == "s-005").unwrap();
            assert_eq!(shared.pr_links.len(), 3);
        }
        // Both memberships span more than one page.
        let confirmed = members
            .iter()
            .filter(|(_, c)| *c != PrConfidence::Inferred)
            .count();
        assert!(confirmed > 50 && confirmed < members.len(), "{confirmed}");
    }

    #[test]
    fn identity_is_exact_across_repositories_numbers_and_casing() {
        let (store, members) = interspersed();
        let listed = |repository: &str, number: u64| {
            let identity = PrIdentity::from_parts(repository, number).unwrap();
            pages(
                &store,
                SessionFilter {
                    pull_request: Some(PrMembership {
                        identity: &identity,
                        confirmed_only: false,
                    }),
                    ..Default::default()
                },
            )
            .0
            .into_iter()
            .map(|row| row.id)
            .collect::<BTreeSet<_>>()
        };
        let expected: BTreeSet<String> = members.into_iter().map(|(id, _)| id).collect();
        // GitHub names are case-insensitive, and the identity is canonical.
        assert_eq!(listed("Example/ATLAS", 7), expected);
        // The same number in another repository, and another number in the
        // same one, are other pull requests.
        let other = listed("other/atlas", 7);
        assert!(!other.is_empty());
        assert!(other.iter().all(|id| {
            // Only members that also link the other repository overlap.
            let index: u32 = id[2..].parse().unwrap();
            !expected.contains(id) || index.is_multiple_of(5)
        }));
        assert_ne!(other, expected);
        assert!(!listed("example/atlas", 8).is_empty());
        assert!(listed("example/atlas", 70).is_empty());
        assert!(listed("example/atlas-7", 7).is_empty());
    }

    #[test]
    fn membership_composes_with_the_other_filters() {
        let (store, members) = interspersed();
        let identity = PrIdentity::from_parts("example/atlas", 7).unwrap();
        let membership = Some(PrMembership {
            identity: &identity,
            confirmed_only: false,
        });
        let codex = ["codex"];
        let rows = pages(
            &store,
            SessionFilter {
                hosts: Some(&codex),
                with_prs: true,
                search: "s-0",
                pull_request: membership,
            },
        )
        .0;
        let expected: BTreeSet<&str> = members
            .iter()
            .map(|(id, _)| id.as_str())
            .filter(|id| id.starts_with("s-0") && id[2..].parse::<u32>().unwrap() % 2 == 1)
            .collect();
        assert!(!expected.is_empty());
        assert_eq!(ids(&rows).into_iter().collect::<BTreeSet<_>>(), expected);
        // Without it, the same filters are unchanged by its existence.
        let without = pages(
            &store,
            SessionFilter {
                hosts: Some(&codex),
                with_prs: true,
                search: "s-0",
                pull_request: None,
            },
        )
        .0;
        assert!(without.len() > rows.len());
    }
}

/// A number GitHub said is not a pull request (its repository resolved, the
/// number did not, and GitHub never confirmed it) is no pull-request link on
/// the Sessions page: not a chip, not a match for "with PRs", and it has no
/// members. A later successful check brings it back.
mod not_found_on_github {
    use xt_store::pr_link::{
        PrConfidence, PrIdentity, PrLinkObservation, PrRefreshError, PrState, RefreshFailure,
        RefreshOutcome, RefreshSuccess,
    };
    use xt_store::session_list::{PrMembership, SessionFilter, SessionSummary};
    use xt_store::{SessionMeta, SessionSource, Store};

    fn link(store: &mut Store, id: &str, number: u64) {
        store
            .record_pr_link(&PrLinkObservation {
                session_id: id.into(),
                pull_request: PrIdentity::from_parts("example/atlas", number).unwrap(),
                confidence: PrConfidence::Exact,
                first_seen_at: 1,
                last_seen_at: 1,
            })
            .unwrap();
    }

    fn answer(store: &mut Store, number: u64, at: i64, error: Option<PrRefreshError>) {
        let pull_request = PrIdentity::from_parts("example/atlas", number).unwrap();
        let outcome = match error {
            Some(error) => RefreshOutcome::Failure(RefreshFailure {
                pull_request,
                attempted_at: at,
                error,
            }),
            None => RefreshOutcome::Success(RefreshSuccess {
                pull_request,
                attempted_at: at,
                title: "Found later".into(),
                state: PrState::Open,
                merged_at: None,
                additions: 1,
                deletions: 1,
                head_ref_name: "feature/later".into(),
            }),
        };
        store.record_pr_refresh(&outcome).unwrap();
    }

    fn chips(rows: &[SessionSummary], id: &str) -> Vec<u64> {
        rows.iter()
            .find(|row| row.id == id)
            .unwrap()
            .pr_links
            .iter()
            .map(|link| link.pull_request.number())
            .collect()
    }

    fn ids(store: &Store, filter: &SessionFilter<'_>) -> Vec<String> {
        let mut ids: Vec<String> = store
            .sessions_page_filtered(filter, None)
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn a_not_found_link_is_no_chip_no_filter_match_and_no_membership() {
        let mut store = Store::open_in_memory().unwrap();
        for id in ["real", "fake-only", "both"] {
            store
                .upsert_session(
                    &SessionMeta::new(id, "claude", SessionSource::Fixture),
                    false,
                )
                .unwrap();
        }
        link(&mut store, "real", 7);
        link(&mut store, "fake-only", 999);
        link(&mut store, "both", 7);
        link(&mut store, "both", 999);
        // A repository gh could not see stays a link.
        link(&mut store, "both", 19);
        answer(&mut store, 999, 10, Some(PrRefreshError::NotFound));
        answer(&mut store, 19, 10, Some(PrRefreshError::ExecutionFailed));

        let all = store
            .sessions_page_filtered(&SessionFilter::default(), None)
            .unwrap();
        assert_eq!(chips(&all, "both"), vec![7, 19]);
        assert!(chips(&all, "fake-only").is_empty());
        let with_prs = SessionFilter {
            with_prs: true,
            ..Default::default()
        };
        assert_eq!(ids(&store, &with_prs), vec!["both", "real"]);
        let fake = PrIdentity::from_parts("example/atlas", 999).unwrap();
        let members = SessionFilter {
            pull_request: Some(PrMembership {
                identity: &fake,
                confirmed_only: false,
            }),
            ..Default::default()
        };
        assert!(ids(&store, &members).is_empty());

        // GitHub later finds it: everything shows it again.
        answer(&mut store, 999, 20, None);
        let all = store
            .sessions_page_filtered(&SessionFilter::default(), None)
            .unwrap();
        assert_eq!(chips(&all, "fake-only"), vec![999]);
        assert_eq!(ids(&store, &with_prs), vec!["both", "fake-only", "real"]);
        assert_eq!(ids(&store, &members), vec!["both", "fake-only"]);
    }
}
