//! The Sessions list's design columns and filters, end to end through the app
//! state the command uses: host set and With PRs applied before the page bound,
//! the cursor carrying the filter, saved titles and recorded starts passed
//! through unchanged (a Claude session without one starts at its earliest
//! message), and a listed row identical to the row named exactly.
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use std::path::Path;
use xt_store::pr_link::{PrConfidence, PrIdentity, PrLinkObservation};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, Store};
use xtrace_desktop::{
    dto::{MetricPrConfidence, MetricSessionHandsOff, MetricSessionWindow, SessionQuery},
    state::{AppState, StartupOptions},
};

fn at(anchor: Timestamp, minutes: i64) -> String {
    (anchor - SignedDuration::from_hours(24) + SignedDuration::from_mins(minutes)).to_string()
}

fn record(uuid: &str, ts: &str, human: bool, tools: usize) -> CanonicalRecord {
    let role = if human { "user" } else { "assistant" };
    let content: Vec<_> = if tools == 0 {
        vec![json!({"type":"text","text":"Synthetic"})]
    } else {
        (0..tools)
            .map(|_| json!({"type":"tool_use","name":"Read","input":{"path":"synthetic"}}))
            .collect()
    };
    serde_json::from_value(json!({"uuid":uuid,"type":role,"timestamp":ts,
        "message":{"role":role,"model":"test-model","content":content}}))
    .unwrap()
}

fn state(root: &Path) -> AppState {
    AppState::build(
        StartupOptions {
            data_dir: Some(root.join("data")),
            native_home: Some(root.join("home")),
            ..Default::default()
        },
        || panic!("explicit data directory"),
        || panic!("explicit native home"),
    )
    .unwrap()
}

#[test]
fn filters_apply_before_the_page_and_listed_rows_match_the_exact_row() {
    let now = Timestamp::now();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    {
        let mut store = Store::open(root.path().join("data/xtrace.db")).unwrap();
        // Fifty-five newer sessions with no link fill more than a page.
        for index in 0..55 {
            let id = format!("plain-{index:02}");
            let mut session = SessionMeta::new(
                &id,
                if index % 2 == 0 { "claude" } else { "cursor" },
                SessionSource::Fixture,
            );
            session.started_at_ms = Some(now.as_millisecond() - 1000 + index);
            store.upsert_session(&session, false).unwrap();
        }
        // Two older linked sessions: one Codex with a native start, one Claude
        // without (shown from its earliest message), each with a measured stretch.
        for (id, host, started, minutes) in [
            ("linked-codex", "codex", Some(0), 0),
            ("linked-claude", "claude", None, 60),
        ] {
            let mut session = SessionMeta::new(id, host, SessionSource::Fixture);
            session.started_at_ms = started;
            store.upsert_session(&session, false).unwrap();
            store
                .upsert_records(
                    id,
                    &[
                        record(&format!("{id}-h"), &at(now, minutes), true, 0),
                        record(&format!("{id}-a"), &at(now, minutes + 3), false, 2),
                    ],
                    false,
                )
                .unwrap();
        }
        for (id, number, confidence) in [
            ("linked-codex", 12, PrConfidence::Inferred),
            ("linked-codex", 11, PrConfidence::Exact),
            ("linked-claude", 11, PrConfidence::Sha),
        ] {
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
    }
    let state = state(root.path());
    let unfiltered = state.sessions_list("", None, None, 7).unwrap();
    assert_eq!(unfiltered.rows.len(), 50);
    assert!(unfiltered.rows.iter().all(|row| row.pr_links.is_empty()));

    let linked = state
        .sessions_query(
            SessionQuery {
                with_prs: true,
                ..Default::default()
            },
            7,
        )
        .unwrap();
    // An unknown native start orders by first recorded work instead, which is
    // yesterday here and so after a start at the epoch.
    assert_eq!(
        linked
            .rows
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["linked-claude", "linked-codex"]
    );
    assert!(linked.next.is_none());
    let codex = &linked.rows[1];
    assert_eq!(codex.started_at_ms, Some(0));
    assert_eq!(codex.title, None);
    assert_eq!(
        codex
            .pr_links
            .iter()
            .map(|l| (l.number, l.confidence))
            .collect::<Vec<_>>(),
        [
            (11, MetricPrConfidence::Exact),
            (12, MetricPrConfidence::Inferred)
        ]
    );
    // Claude Code records no start: a Claude session starts at its earliest
    // message.
    assert_eq!(
        linked.rows[0].started_at_ms,
        Some(at(now, 60).parse::<Timestamp>().unwrap().as_millisecond())
    );
    let MetricSessionWindow::Indexed { tool_calls, .. } = &codex.metrics else {
        panic!("indexed");
    };
    assert_eq!(*tool_calls, Some(2));
    assert_eq!(
        codex.hands_off,
        MetricSessionHandsOff::Measured {
            n: 1,
            median_min: Some(3.0)
        }
    );

    // The host set narrows the same filtered list, before paging.
    let hosts = ["claude", "codex"];
    let narrowed = state
        .sessions_query(
            SessionQuery {
                hosts: Some(&hosts),
                ..Default::default()
            },
            7,
        )
        .unwrap();
    assert!(narrowed.rows.iter().all(|r| r.host != "cursor"));
    let mut total = narrowed.rows.len();
    let mut next = narrowed.next.clone();
    // The cursor continues under the same filter.
    while let Some(cursor) = next {
        let page = state
            .sessions_query(
                SessionQuery {
                    hosts: Some(&hosts),
                    after: Some(&cursor),
                    ..Default::default()
                },
                7,
            )
            .unwrap();
        assert!(page.rows.iter().all(|r| r.host != "cursor"));
        total += page.rows.len();
        next = page.next;
    }
    assert_eq!(total, 28 + 2);
    let bad = ["claude", "nope"];
    assert!(
        state
            .sessions_query(
                SessionQuery {
                    hosts: Some(&bad),
                    ..Default::default()
                },
                7
            )
            .is_err()
    );

    // The detail's exact row is the listed row, field for field.
    for listed in &linked.rows {
        assert_eq!(&state.session_row(&listed.id, 7).unwrap().unwrap(), listed);
    }
}

/// Cost of one list page on a synthetic history, for the record rather than as
/// a gate: `cargo test -p xtrace-desktop --release --test session_list_design
/// -- --ignored --nocapture`. The page reads one bounded metadata page, one
/// link statement, the page's own events and usage, and a single pass over the
/// window's events for hands-off surface health. Its first page also measures
/// the summary across every matching indexed session.
#[test]
#[ignore]
fn list_page_cost_on_a_synthetic_history() {
    let now = Timestamp::now();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    const SESSIONS: usize = 2_000;
    const TURNS: usize = 20;
    {
        let mut store = Store::open(root.path().join("data/xtrace.db")).unwrap();
        for index in 0..SESSIONS {
            let id = format!("synthetic-{index:05}");
            let mut session = SessionMeta::new(
                &id,
                ["claude", "codex", "cursor"][index % 3],
                SessionSource::Fixture,
            );
            session.started_at_ms = Some(now.as_millisecond() - index as i64 * 60_000);
            store.upsert_session(&session, false).unwrap();
            let mut rows = Vec::new();
            for turn in 0..TURNS {
                let minute = (index % 1_000) as i64 + turn as i64 * 2;
                rows.push(record(&format!("{id}-h{turn}"), &at(now, minute), true, 0));
                rows.push(record(
                    &format!("{id}-a{turn}"),
                    &at(now, minute + 1),
                    false,
                    1,
                ));
            }
            store.upsert_records(&id, &rows, false).unwrap();
            if index % 10 == 0 {
                store
                    .record_pr_link(&PrLinkObservation {
                        session_id: id,
                        pull_request: PrIdentity::from_parts("example/atlas", index as u64 + 1)
                            .unwrap(),
                        confidence: PrConfidence::Exact,
                        first_seen_at: 1,
                        last_seen_at: 1,
                    })
                    .unwrap();
            }
        }
    }
    let state = state(root.path());
    let hosts = ["codex"];
    for (name, query) in [
        ("all", SessionQuery::default()),
        (
            "with PRs",
            SessionQuery {
                with_prs: true,
                ..Default::default()
            },
        ),
        (
            "codex",
            SessionQuery {
                hosts: Some(&hosts),
                ..Default::default()
            },
        ),
        (
            "search",
            SessionQuery {
                search: "01999",
                ..Default::default()
            },
        ),
    ] {
        for days in [7, 30] {
            let started = std::time::Instant::now();
            let page = state.sessions_query(query, days).unwrap();
            println!(
                "{name:>8} {days:>2}d: {:>3} rows in {:?} ({} sessions × {} records)",
                page.rows.len(),
                started.elapsed(),
                SESSIONS,
                TURNS * 2
            );
        }
    }
}

#[test]
fn only_the_first_page_carries_all_matching_summary_totals() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    {
        let mut store = Store::open(root.path().join("data/xtrace.db")).unwrap();
        for index in 0..65 {
            let mut session =
                SessionMeta::new(format!("main-{index:02}"), "claude", SessionSource::Fixture);
            session.started_at_ms = Some(index);
            store.upsert_session(&session, false).unwrap();
        }
    }
    let state = state(root.path());
    let first = state.sessions_list("", None, None, 7).unwrap();
    let totals = first.summary.unwrap();
    assert_eq!(totals.main_sessions + totals.checking_sessions, 65);
    assert_eq!(totals.human_messages, Some(0));
    let second = state
        .sessions_list("", None, first.next.as_deref(), 7)
        .unwrap();
    assert_eq!(second.rows.len(), 15);
    assert!(second.summary.is_none());
    let narrowed = state.sessions_list("main-00", None, None, 7).unwrap();
    let totals = narrowed.summary.unwrap();
    assert_eq!(totals.main_sessions + totals.checking_sessions, 1);
}

#[test]
fn a_failed_later_session_summary_does_not_discard_valid_first_page_rows() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    let path = root.path().join("data/xtrace.db");
    let now = Timestamp::now();
    {
        let mut store = Store::open(&path).unwrap();
        for index in 0..65 {
            let mut session = SessionMeta::new(
                format!("session-{index:02}"),
                "codex",
                SessionSource::Fixture,
            );
            session.started_at_ms = Some(index);
            store.upsert_session(&session, false).unwrap();
        }
        store
            .upsert_records(
                "session-00",
                &[serde_json::from_value(json!({
                    "uuid": "broken-later-record", "type": "assistant", "timestamp": at(now, 60),
                    "message": {"role": "assistant", "id": "broken-response", "content": [],
                        "usage": {"input_tokens": 1, "output_tokens": 2,
                            "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}
                }))
                .unwrap()],
                false,
            )
            .unwrap();
    }
    // Synthetic damaged metric data in the oldest row, beyond the first page.
    // Rows on that page remain readable; aggregate measurement must fail.
    let connection = rusqlite::Connection::open(&path).unwrap();
    xt_store::timestamp::register_sqlite(&connection).unwrap();
    connection
        .execute_batch("PRAGMA ignore_check_constraints=ON;")
        .unwrap();
    connection
        .execute(
            "UPDATE usage SET input_tokens=-1 WHERE uuid='broken-later-record'",
            [],
        )
        .unwrap();
    let state = state(root.path());
    let first = state.sessions_list("", None, None, 7).unwrap();
    assert_eq!(first.rows.len(), 50);
    assert!(first.summary.is_none());
    assert!(first.next.is_some());
}
