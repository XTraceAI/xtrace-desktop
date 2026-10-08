use serde_json::json;
use xt_fixtures::TempDb;
use xt_metrics::{MetricsDb, Window};
use xt_store::pr_link::{PrConfidence, PrIdentity, PrLinkObservation};
use xt_store::session_list::{SessionFilter, SessionSort};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource};

fn record(id: &str, time: &str, human: bool) -> CanonicalRecord {
    serde_json::from_value(
        json!({"uuid": id, "type": if human {"user"} else {"assistant"},
        "timestamp": time, "message": {"role": if human {"user"} else {"assistant"},
        "content": [{"type":"text","text":"Synthetic"}]}}),
    )
    .unwrap()
}

#[test]
fn totals_follow_all_filtered_pages_and_the_existing_window_rules() {
    let mut db = TempDb::empty().unwrap();
    for index in 0..123 {
        let id = format!("session-{index:03}");
        let mut session = SessionMeta::new(
            &id,
            if index % 2 == 0 { "claude" } else { "cursor" },
            SessionSource::Fixture,
        );
        session.cwd = Some(format!(
            "/repo/{}",
            if index < 60 { "atlas" } else { "other" }
        ));
        session.started_at_ms = Some(index);
        db.store_mut().upsert_session(&session, false).unwrap();
        db.store_mut()
            .upsert_records(
                &id,
                &[
                    record(&format!("{id}-h"), "2026-09-07T12:00:00Z", true),
                    record(&format!("{id}-a"), "2026-09-07T12:03:00Z", false),
                    record(&format!("{id}-old"), "2026-08-01T00:00:00Z", true),
                ],
                false,
            )
            .unwrap();
        if index % 3 == 0 {
            for number in [1, 2] {
                db.store_mut()
                    .record_pr_link(&PrLinkObservation {
                        session_id: id.clone(),
                        pull_request: PrIdentity::from_parts("example/atlas", number).unwrap(),
                        confidence: PrConfidence::Inferred,
                        first_seen_at: 1,
                        last_seen_at: 1,
                    })
                    .unwrap();
            }
        }
    }
    let metrics = MetricsDb::open(db.path()).unwrap();
    let window = Window::new(
        "2026-09-07T00:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond(),
        "2026-09-08T00:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond(),
    )
    .unwrap();
    for sort in [SessionSort::Started, SessionSort::RecentlyActive] {
        let filter = SessionFilter {
            sort,
            ..Default::default()
        };
        assert_eq!(
            metrics.sessions_page_filtered(&filter, None).unwrap().len(),
            51
        );
        let total = metrics.sessions_summary(window, &filter).unwrap();
        assert_eq!(total.main_sessions + total.checking_sessions, 123);
        assert_eq!(total.human_messages, Some(123));
        assert_eq!(total.agent_ms, Some(123 * 180_000));
        assert_eq!(total.sessions_with_prs, 41);
    }
    let filter = SessionFilter {
        search: "atlas",
        hosts: Some(&["claude"]),
        with_prs: true,
        ..Default::default()
    };
    let total = metrics.sessions_summary(window, &filter).unwrap();
    assert_eq!(total.main_sessions + total.checking_sessions, 10);
    assert_eq!(total.human_messages, Some(10));
    assert_eq!(total.agent_ms, Some(10 * 180_000));
    assert_eq!(total.sessions_with_prs, 10); // Two links still count one session.
    let quiet = metrics
        .sessions_summary(Window::new(0, 1).unwrap(), &filter)
        .unwrap();
    assert_eq!(quiet.human_messages, Some(0));
    assert_eq!(quiet.agent_ms, Some(0));
    assert_eq!(quiet.sessions_with_prs, 10); // Membership is not event-window membership.
    let empty = metrics
        .sessions_summary(
            window,
            &SessionFilter {
                search: "absent",
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(empty.human_messages, Some(0));
    assert_eq!(empty.main_sessions, 0);
    assert_eq!(empty.messages_per_main_session, None);
}

#[test]
fn missing_message_classification_is_unknown_instead_of_zero() {
    let mut db = TempDb::empty().unwrap();
    db.store_mut()
        .upsert_session(
            &SessionMeta::new("unknown", "claude", SessionSource::Fixture),
            false,
        )
        .unwrap();
    let unknown = record("unknown-message", "2026-09-07T12:00:00Z", true);
    db.store_mut()
        .upsert_records("unknown", &[unknown], false)
        .unwrap();
    let connection = rusqlite::Connection::open(db.path()).unwrap();
    xt_store::timestamp::register_sqlite(&connection).unwrap();
    connection
        .execute("UPDATE records SET is_human=NULL", [])
        .unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let total = metrics
        .sessions_summary(
            Window::new(0, 2_000_000_000_000).unwrap(),
            &SessionFilter::default(),
        )
        .unwrap();
    assert_eq!(total.human_messages, None);
    assert_eq!(total.messages_per_main_session, None);
    assert_eq!(total.agent_ms, Some(0));
}

#[test]
fn classifications_separate_checked_main_children_and_pending_checks() {
    use xt_store::{
        Host,
        child_fact::{ChildEvidence, ChildFact},
        creation::{
            CODEX_THREAD_SPAWN_VERSION, CreationEvidence, CreationWitness, SessionCreationProof,
        },
    };
    let mut db = TempDb::empty().unwrap();
    let key = "x1:0000000000000000000000000000000000000000000000000000000000000001";
    for native in ["main", "child", "orphan", "pending"] {
        let id = format!("codex-{native}");
        let mut session = SessionMeta::new(&id, "codex", SessionSource::ReadersCli);
        session.native_session_id = Some(native.into());
        db.store_mut().upsert_session(&session, false).unwrap();
        db.store_mut()
            .upsert_records(
                &id,
                &[record(&format!("{id}-h"), "2026-09-07T12:00:00Z", true)],
                false,
            )
            .unwrap();
    }
    db.store_mut()
        .observe_own_check("codex-main", Some(key))
        .unwrap();
    db.store_mut()
        .settle_child_checks(&[("codex-main", 1, key)])
        .unwrap();
    db.store_mut()
        .record_child_facts(
            &[ChildFact::typed(
                "codex-orphan",
                "orphan",
                ChildEvidence::CodexThreadSpawn,
            )],
            1,
        )
        .unwrap();
    db.store_mut()
        .record_session_creations(
            &[SessionCreationProof {
                child_session_id: "codex-child".into(),
                child_host: Host::Codex,
                child_native_session_id: "child".into(),
                parent_host: Host::Codex,
                parent_native_session_id: "main".into(),
                evidence_kind: CreationEvidence::CodexThreadSpawn,
                evidence_version: CODEX_THREAD_SPAWN_VERSION,
                witness: CreationWitness::RolloutOpeningSessionMeta,
            }],
            1,
        )
        .unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let window = Window::new(0, 2_000_000_000_000).unwrap();
    let total = metrics
        .sessions_summary(window, &SessionFilter::default())
        .unwrap();
    assert_eq!(
        (
            total.main_sessions,
            total.sub_sessions,
            total.checking_sessions,
            total.unlinked_sub_sessions
        ),
        (1, 2, 1, 1)
    );
    assert_eq!(total.human_messages, Some(3)); // Shared M-02 excludes the verified spawned first input.
    assert_eq!(total.messages_per_main_session, None);
    db.store_mut()
        .observe_own_check("codex-pending", Some(key))
        .unwrap();
    db.store_mut()
        .settle_child_checks(&[("codex-pending", 1, key)])
        .unwrap();
    let total = metrics
        .sessions_summary(window, &SessionFilter::default())
        .unwrap();
    assert_eq!(total.main_sessions, 2);
    assert_eq!(total.messages_per_main_session, Some(1.5));
    let child = metrics
        .sessions_summary(
            window,
            &SessionFilter {
                search: "codex-child",
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!((child.main_sessions, child.sub_sessions), (0, 1)); // Parent outside filter is not counted.
    assert_eq!(child.messages_per_main_session, None);
}
