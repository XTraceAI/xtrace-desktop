//! The PRs page's two reads end to end through the app state their commands
//! call: the report is exactly the core report for the preset and confidence
//! mode, and one pull request's drilldown lists exactly its linked members,
//! filtered before the page bound, over the report's own pinned window.
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use std::collections::BTreeSet;
use std::path::Path;
use xt_store::pr_link::{
    PrConfidence, PrIdentity, PrLinkObservation, PrState, RefreshOutcome, RefreshSuccess,
};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, Store};
use xtrace_desktop::{
    dto::{MetricSessionWindow, PrAnalyticsPage, SessionPage},
    pr_analytics::PrSessionsRequest,
    state::{AppState, StartupOptions},
};

const DAY_MS: i64 = 86_400_000;

fn record(uuid: &str, ts: &str, human: bool) -> CanonicalRecord {
    let value = if human {
        json!({"uuid":uuid,"type":"user","timestamp":ts,
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic"}]}})
    } else {
        json!({"uuid":uuid,"type":"assistant","timestamp":ts,
            "message":{"role":"assistant","model":"test-model",
                "content":[{"type":"tool_use","name":"Read","input":{"path":"synthetic"}}],
                "usage":{"input_tokens":10,"output_tokens":5,
                         "cache_read_input_tokens":2,"cache_creation_input_tokens":1}}})
    };
    serde_json::from_value(value).unwrap()
}

fn session(store: &mut Store, id: &str, started_at_ms: Option<i64>) {
    let mut session = SessionMeta::new(id, "claude", SessionSource::Fixture);
    session.started_at_ms = started_at_ms;
    store.upsert_session(&session, false).unwrap();
}

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

fn refresh(store: &mut Store, repository: &str, number: u64, title: &str, merged: Option<String>) {
    store
        .record_pr_refresh(&RefreshOutcome::Success(RefreshSuccess {
            pull_request: PrIdentity::from_parts(repository, number).unwrap(),
            attempted_at: 1,
            title: title.into(),
            state: if merged.is_some() {
                PrState::Merged
            } else {
                PrState::Open
            },
            merged_at: merged,
            additions: 1,
            deletions: 1,
            head_ref_name: "synthetic".into(),
        }))
        .unwrap();
}

/// Three measured sessions and an idle one around five pull requests, plus
/// 56 members of `example/atlas#2` interleaved with 55 unlinked sessions that
/// share their native starts:
///
/// - `example/atlas#1`, merged yesterday: `s-active` exact, `s-idle` (no
///   records) inferred, `s-shared` SHA.
/// - `example/atlas#2`, merged two days ago: `s-shared` exact and the bulk.
/// - `other/atlas#1`, merged yesterday: `s-other` exact. Same number, other
///   repository.
/// - `example/atlas#4`, open: `s-active` inferred.
/// - `example/atlas#5`, never refreshed: `s-shared` exact.
fn seeded(root: &Path, now: Timestamp) -> AppState {
    std::fs::create_dir_all(root.join("home")).unwrap();
    std::fs::create_dir_all(root.join("data")).unwrap();
    let yesterday = now - SignedDuration::from_hours(24);
    let at = |minutes: i64| (yesterday + SignedDuration::from_mins(minutes)).to_string();
    {
        let mut store = Store::open(root.join("data/xtrace.db")).unwrap();
        for (id, minutes) in [("s-active", 0), ("s-shared", 30), ("s-other", 60)] {
            session(&mut store, id, Some(yesterday.as_millisecond() + minutes));
            store
                .upsert_records(
                    id,
                    &[
                        record(&format!("{id}-h"), &at(minutes), true),
                        record(&format!("{id}-a"), &at(minutes + 2), false),
                    ],
                    false,
                )
                .unwrap();
        }
        session(&mut store, "s-idle", None);
        for index in 0..56_i64 {
            let started = Some(now.as_millisecond() - 10_000 + index / 2 * 7);
            session(&mut store, &format!("bulk-{index:02}"), started);
            link(
                &mut store,
                &format!("bulk-{index:02}"),
                "example/atlas",
                2,
                PrConfidence::Exact,
            );
            if index < 55 {
                session(&mut store, &format!("noise-{index:02}"), started);
            }
        }
        link(
            &mut store,
            "s-active",
            "example/atlas",
            1,
            PrConfidence::Exact,
        );
        link(
            &mut store,
            "s-idle",
            "example/atlas",
            1,
            PrConfidence::Inferred,
        );
        link(
            &mut store,
            "s-shared",
            "example/atlas",
            1,
            PrConfidence::Sha,
        );
        link(
            &mut store,
            "s-shared",
            "example/atlas",
            2,
            PrConfidence::Exact,
        );
        link(&mut store, "s-other", "other/atlas", 1, PrConfidence::Exact);
        link(
            &mut store,
            "s-active",
            "example/atlas",
            4,
            PrConfidence::Inferred,
        );
        link(
            &mut store,
            "s-shared",
            "example/atlas",
            5,
            PrConfidence::Exact,
        );
        let merged = |days: i64| Some((now - SignedDuration::from_hours(24 * days)).to_string());
        refresh(&mut store, "example/atlas", 1, "feat: one", merged(1));
        refresh(&mut store, "example/atlas", 2, "fix: two", merged(2));
        refresh(&mut store, "other/atlas", 1, "feat: other", merged(1));
        refresh(&mut store, "example/atlas", 4, "chore: open", None);
    }
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

fn row<'a>(
    page: &'a PrAnalyticsPage,
    repository: &str,
    number: u64,
) -> Option<&'a xtrace_desktop::dto::MetricPrAnalyticsRow> {
    page.report
        .rows
        .iter()
        .find(|row| row.repository == repository && row.number == number)
}

fn request<'a>(
    report: &PrAnalyticsPage,
    repository: &'a str,
    number: u64,
    after: Option<&'a str>,
) -> PrSessionsRequest<'a> {
    PrSessionsRequest {
        repository,
        number,
        confirmed_only: report.report.confirmed_only,
        window_days: report.window.days,
        window_end_ms: report.window.end_ms,
        after,
    }
}

/// Every page of one drilldown, following `next`.
fn all_pages(
    state: &AppState,
    report: &PrAnalyticsPage,
    repository: &str,
    number: u64,
) -> Vec<SessionPage> {
    let mut pages = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let page = state
            .pr_sessions(request(report, repository, number, after.as_deref()))
            .unwrap();
        after = page.next.clone();
        pages.push(page);
        if after.is_none() {
            return pages;
        }
    }
}

fn ids(pages: &[SessionPage]) -> Vec<String> {
    pages
        .iter()
        .flat_map(|page| page.rows.iter().map(|row| row.id.clone()))
        .collect()
}

#[test]
fn the_report_is_the_core_report_for_every_preset_and_confidence_mode() {
    let root = tempfile::tempdir().unwrap();
    let state = seeded(root.path(), Timestamp::now());
    let metrics = xt_metrics::MetricsDb::open(root.path().join("data/xtrace.db")).unwrap();
    for days in [7, 14, 30] {
        for confirmed_only in [false, true] {
            let page = state.pr_analytics(days, confirmed_only).unwrap();
            // The window ends at the one instant the gate is anchored to.
            assert_eq!(page.window.days, days);
            assert_eq!(page.window.end_ms, page.report.now_ms);
            assert_eq!(
                page.window.start_ms,
                page.window.end_ms - i64::from(days) * DAY_MS
            );
            assert_eq!(page.report.confirmed_only, confirmed_only);
            // Field for field the core's answer for that window and instant.
            let core = metrics
                .pr_analytics(
                    xt_metrics::Window::new(page.window.start_ms, page.window.end_ms).unwrap(),
                    page.report.now_ms,
                    confirmed_only,
                )
                .unwrap();
            assert_eq!(
                serde_json::to_value(&page.report).unwrap(),
                serde_json::to_value(&core).unwrap(),
                "{days}d confirmed_only={confirmed_only}"
            );
        }
    }

    // What that parity carries across: the idle inferred member keeps the
    // row's tokens unknown and is still a linked member; the shared session
    // counts in both rows; the unrefreshed and open pull requests are not rows.
    let all = state.pr_analytics(7, false).unwrap();
    let one = row(&all, "example/atlas", 1).unwrap();
    assert_eq!((one.linked_sessions, one.active_sessions), (3, 2));
    assert_eq!(one.tokens.counters.total_tokens, None);
    assert_eq!(one.tokens.no_selected_usage_sessions, 1);
    assert_eq!(one.human_messages, Some(2));
    assert_eq!(
        (one.evidence.exact, one.evidence.sha, one.evidence.inferred),
        (1, 1, 1)
    );
    let two = row(&all, "example/atlas", 2).unwrap();
    assert_eq!((two.linked_sessions, two.active_sessions), (57, 1));
    assert!(row(&all, "other/atlas", 1).is_some());
    assert!(row(&all, "example/atlas", 4).is_none());
    assert!(row(&all, "example/atlas", 5).is_none());
    assert_eq!(
        (
            all.report.eligibility.merged,
            all.report.eligibility.outside,
            all.report.eligibility.unknown_facts
        ),
        (3, 1, 1)
    );
    // Token medians wait for the fixed gate; the counts are always there.
    assert_eq!(all.report.summary.tokens.median.eligible_prs, 3);

    let confirmed = state.pr_analytics(7, true).unwrap();
    let one = row(&confirmed, "example/atlas", 1).unwrap();
    assert_eq!((one.linked_sessions, one.active_sessions), (2, 2));
    assert_eq!(one.tokens.counters.total_tokens, Some(36));
    // The open pull request's only link was inferred, so it is gone entirely.
    assert_eq!(confirmed.report.eligibility.outside, 0);
    state.shutdown();
}

#[test]
fn a_drilldown_lists_exact_linked_members_over_the_pinned_report_window() {
    let root = tempfile::tempdir().unwrap();
    let state = seeded(root.path(), Timestamp::now());
    let all = state.pr_analytics(7, false).unwrap();
    let confirmed = state.pr_analytics(7, true).unwrap();
    // The clock moves on between the report, the click and the next page.
    std::thread::sleep(std::time::Duration::from_millis(20));

    let pages = all_pages(&state, &all, "example/atlas", 1);
    assert_eq!(pages.len(), 1);
    let page = &pages[0];
    assert_eq!(
        page.window, all.window,
        "the report's window, not a new one"
    );
    let members: BTreeSet<String> = ids(&pages).into_iter().collect();
    assert_eq!(
        members,
        ["s-active", "s-idle", "s-shared"].map(String::from).into()
    );
    assert_eq!(
        members.len() as u64,
        row(&all, "example/atlas", 1).unwrap().linked_sessions
    );
    // Linked membership, not activity: the idle member is listed with its
    // empty window, its unknown tokens and its own inferred link.
    let idle = page.rows.iter().find(|row| row.id == "s-idle").unwrap();
    match &idle.metrics {
        MetricSessionWindow::Indexed { events, tokens, .. } => {
            assert_eq!(*events, 0);
            assert_eq!(tokens.counters.total_tokens, None);
        }
        other => panic!("{other:?}"),
    }
    let selected = idle
        .pr_links
        .iter()
        .find(|link| link.repository == "example/atlas" && link.number == 1)
        .unwrap();
    assert_eq!(
        serde_json::to_value(selected.confidence).unwrap(),
        "inferred"
    );

    // Confirmed only drops the inferred-only member, as the report does, and
    // the listed members' own totals are the report row's.
    let pages = all_pages(&state, &confirmed, "example/atlas", 1);
    let members: BTreeSet<String> = ids(&pages).into_iter().collect();
    assert_eq!(members, ["s-active", "s-shared"].map(String::from).into());
    let summed: u64 = pages[0]
        .rows
        .iter()
        .map(|row| match &row.metrics {
            MetricSessionWindow::Indexed { tokens, .. } => tokens.counters.total_tokens.unwrap(),
            other => panic!("{other:?}"),
        })
        .sum();
    assert_eq!(
        Some(summed),
        row(&confirmed, "example/atlas", 1)
            .unwrap()
            .tokens
            .counters
            .total_tokens
    );

    // Canonical casing names the same pull request; the same number in
    // another repository does not.
    assert_eq!(
        ids(&all_pages(&state, &all, "Example/ATLAS", 1)),
        ids(&all_pages(&state, &all, "example/atlas", 1))
    );
    assert_eq!(ids(&all_pages(&state, &all, "other/atlas", 1)), ["s-other"]);

    // More than a page of members among as many non-members with the same
    // starts: every member once, every page over the same pinned window.
    let pages = all_pages(&state, &all, "example/atlas", 2);
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].rows.len(), 50);
    let listed = ids(&pages);
    let unique: BTreeSet<&String> = listed.iter().collect();
    assert_eq!(unique.len(), listed.len(), "no duplicates");
    assert_eq!(
        listed.len() as u64,
        row(&all, "example/atlas", 2).unwrap().linked_sessions
    );
    assert!(
        listed
            .iter()
            .all(|id| id.starts_with("bulk-") || id == "s-shared")
    );
    assert!(pages.iter().all(|page| page.window == all.window));

    // Another read captures a later clock; the drilldown still does not.
    let later = state.sessions_list("", None, None, 7).unwrap();
    assert!(later.window.end_ms > all.window.end_ms);
    let again = state
        .pr_sessions(request(&all, "example/atlas", 1, None))
        .unwrap();
    assert_eq!(again.window, all.window);

    // A pull request with no stored links has no members, not an error.
    assert!(
        state
            .pr_sessions(request(&all, "example/atlas", 99, None))
            .unwrap()
            .rows
            .is_empty()
    );
    state.shutdown();
}

#[test]
fn requests_are_checked_before_storage_is_touched() {
    let root = tempfile::tempdir().unwrap();
    let state = seeded(root.path(), Timestamp::now());
    let report = state.pr_analytics(7, false).unwrap();
    state.shutdown();
    // Refused for what they say, even though the database is already closed.
    let refused =
        |request: PrSessionsRequest<'_>| state.pr_sessions(request).unwrap_err().to_string();
    let valid = request(&report, "example/atlas", 1, None);
    assert_eq!(refused(valid), "application database is closed");
    assert_eq!(
        refused(PrSessionsRequest {
            window_days: 8,
            ..valid
        }),
        "metric range must be 7, 14, or 30 days"
    );
    assert_eq!(
        refused(PrSessionsRequest {
            repository: "example/atlas/pull/1",
            ..valid
        }),
        "invalid pull-request session request: pull request identity"
    );
    assert_eq!(
        refused(PrSessionsRequest {
            number: 1 << 53,
            ..valid
        }),
        "invalid pull-request session request: pull request number"
    );
    assert_eq!(
        refused(PrSessionsRequest {
            window_end_ms: 1 << 53,
            ..valid
        }),
        "invalid pull-request session request: window anchor"
    );
    assert_eq!(
        refused(PrSessionsRequest {
            after: Some("{\"time\":1}"),
            ..valid
        }),
        "application storage operation failed"
    );
    assert_eq!(
        state.pr_analytics(8, false).unwrap_err().to_string(),
        "metric range must be 7, 14, or 30 days"
    );
    assert_eq!(
        state.pr_analytics(7, false).unwrap_err().to_string(),
        "application database is closed"
    );
}

/// Test-only synthetic data for the PRs page's UI acceptance: the reports and
/// the linked-session pages the app's own commands assemble over a seeded
/// database at F1's pinned instant. F1's only merged pull request merges at
/// its window's exclusive end, so its reports have no rows; this export is the
/// non-empty counterpart. Not real history and not a design sample.
mod ui_export {
    use super::*;
    use jiff::tz::TimeZone;
    use serde::Serialize;
    use xt_store::pr_link::{PrRefreshError, RefreshFailure};
    use xtrace_desktop::{
        dto::{FixturePrSessions, MetricClock},
        pr_analytics::{fixture_pages, sessions, validate},
    };

    /// F1's pinned instant, so the Shell's F1 window and these reports agree.
    const NOW: &str = "2026-09-08T00:00:00Z";
    const MODEL: &str = "claude-synthetic";
    const EXPORT: &str = "../ui/src/app/prs-analytics.synthetic.json";

    fn ts(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn human(uuid: &str, at: Timestamp) -> CanonicalRecord {
        serde_json::from_value(json!({"uuid":uuid,"type":"user","timestamp":at.to_string(),
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic"}]}}))
        .unwrap()
    }

    fn plain(uuid: &str, at: Timestamp) -> CanonicalRecord {
        serde_json::from_value(
            json!({"uuid":uuid,"type":"assistant","timestamp":at.to_string(),
            "message":{"role":"assistant","content":[{"type":"text","text":"Synthetic"}]}}),
        )
        .unwrap()
    }

    /// A tool call carrying one response's usage: input 1, `output`, zero
    /// cache counters, and the model unless `model` is false.
    fn tool(uuid: &str, at: Timestamp, output: u64, model: bool) -> CanonicalRecord {
        let mut message = json!({"role":"assistant",
            "content":[{"type":"tool_use","id":format!("tool-{uuid}"),"name":"Read","input":{"file_path":"synthetic"}}],
            "usage":{"input_tokens":1,"output_tokens":output,
                     "cache_read_input_tokens":0,"cache_creation_input_tokens":0}});
        if model {
            message["model"] = json!(MODEL);
        }
        serde_json::from_value(
            json!({"uuid":uuid,"type":"assistant","timestamp":at.to_string(),"message":message}),
        )
        .unwrap()
    }

    /// Human messages each answered `minutes` later by a measured tool call.
    fn stretches(id: &str, stretches: &[(&str, i64, u64)]) -> Vec<CanonicalRecord> {
        stretches
            .iter()
            .enumerate()
            .flat_map(|(index, (at, minutes, output))| {
                let start = ts(at);
                [
                    human(&format!("{id}-h{index}"), start),
                    tool(
                        &format!("{id}-t{index}"),
                        start + SignedDuration::from_mins(*minutes),
                        *output,
                        true,
                    ),
                ]
            })
            .collect()
    }

    struct Seed {
        store: Store,
    }

    impl Seed {
        fn session(&mut self, id: &str, surface: &str, rows: &[CanonicalRecord]) -> &mut Self {
            let mut session = SessionMeta::new(id, "claude", SessionSource::Fixture);
            session.surface = Some(surface.into());
            session.started_at_ms = rows
                .iter()
                .filter_map(|row| row.timestamp.as_deref())
                .map(|at| ts(at).as_millisecond())
                .min();
            self.store.upsert_session(&session, false).unwrap();
            if !rows.is_empty() {
                self.store.upsert_records(id, rows, false).unwrap();
            }
            self
        }

        fn link(&mut self, id: &str, pr: (&str, u64), confidence: PrConfidence) -> &mut Self {
            link(&mut self.store, id, pr.0, pr.1, confidence);
            self
        }

        fn merged(&mut self, pr: (&str, u64), title: &str, head: &str, at: &str) -> &mut Self {
            self.facts(pr, title, head, PrState::Merged, Some(at))
        }

        fn facts(
            &mut self,
            (repository, number): (&str, u64),
            title: &str,
            head: &str,
            state: PrState,
            merged_at: Option<&str>,
        ) -> &mut Self {
            self.store
                .record_pr_refresh(&RefreshOutcome::Success(RefreshSuccess {
                    pull_request: PrIdentity::from_parts(repository, number).unwrap(),
                    attempted_at: ts("2026-09-07T20:00:00Z").as_millisecond(),
                    title: title.into(),
                    state,
                    merged_at: merged_at.map(Into::into),
                    additions: 1,
                    deletions: 1,
                    head_ref_name: head.into(),
                }))
                .unwrap();
            self
        }

        fn failed(
            &mut self,
            (repository, number): (&str, u64),
            error: PrRefreshError,
        ) -> &mut Self {
            self.store
                .record_pr_refresh(&RefreshOutcome::Failure(RefreshFailure {
                    pull_request: PrIdentity::from_parts(repository, number).unwrap(),
                    attempted_at: ts("2026-09-07T21:00:00Z").as_millisecond(),
                    error,
                }))
                .unwrap();
            self
        }
    }

    fn raw(path: &Path, sql: &str) {
        let connection = rusqlite::Connection::open(path).unwrap();
        xt_store::timestamp::register_sqlite(&connection).unwrap();
        connection.execute_batch(sql).unwrap();
    }

    const ATLAS: &str = "example/atlas";
    const HARBOR: &str = "example/harbor";

    /// Every state a row, a summary and a drilldown must keep apart, at 7 days
    /// unless noted: overlapping members, a shared session in three rows, an
    /// idle inferred member (tokens unknown), a judge identity (unmeasured
    /// link), measured-zero human messages and measured-zero tokens, an
    /// unknown human classification, an unresolved work type, a surface
    /// excluded by timestamp health, a row without stretches, facts kept after
    /// a failed refresh, 52 members over two pages, a row only 14 and 30 days
    /// hold, an open, a never-refreshed and a never-successfully-refreshed
    /// pull request.
    fn measured(path: &Path) -> Vec<(String, u64)> {
        let mut seed = Seed {
            store: Store::open(path).unwrap(),
        };
        let alpha = stretches(
            "s-alpha",
            &[
                ("2026-09-06T10:00:00Z", 3, 400),
                ("2026-09-06T10:10:00Z", 5, 250),
                ("2026-09-06T10:30:00Z", 2, 120),
            ],
        );
        let shared = stretches(
            "s-shared",
            &[
                ("2026-09-05T09:00:00Z", 4, 900),
                ("2026-09-05T09:20:00Z", 6, 300),
            ],
        );
        let tools_only: Vec<_> = (0..2)
            .map(|index| {
                tool(
                    &format!("s-tools-t{index}"),
                    ts("2026-09-04T08:00:00Z") + SignedDuration::from_mins(5 * index),
                    50,
                    true,
                )
            })
            .collect();
        let degenerate = |id: &str| -> Vec<CanonicalRecord> {
            (0..6)
                .map(|index| plain(&format!("{id}-{index}"), ts("2026-09-06T07:00:00Z")))
                .collect()
        };
        seed.session("s-alpha", "cli", &alpha)
            .session("s-shared", "cli", &shared)
            .session(
                "s-beta",
                "cli",
                &stretches("s-beta", &[("2026-09-06T14:00:00Z", 1, 80)]),
            )
            .session("s-idle", "cli", &[])
            .session(
                "s-judge",
                "cli",
                &stretches("s-judge", &[("2026-09-06T15:00:00Z", 1, 5)]),
            )
            .session("s-tools", "cli", &tools_only)
            .session(
                "s-zero",
                "cli",
                &stretches("s-zero", &[("2026-09-07T08:00:00Z", 2, 0)]),
            )
            .session(
                "s-unclassified",
                "cli",
                &stretches("s-unclassified", &[("2026-09-03T11:00:00Z", 3, 60)]),
            )
            .session(
                "s-desk",
                "desktop",
                &stretches(
                    "s-desk",
                    &[
                        ("2026-09-06T06:00:00Z", 1, 20),
                        ("2026-09-06T06:10:00Z", 2, 20),
                        ("2026-09-06T06:20:00Z", 3, 20),
                    ],
                ),
            )
            .session("d-one", "desktop", &degenerate("d-one"))
            .session("d-two", "desktop", &degenerate("d-two"))
            .session(
                "s-failed",
                "cli",
                &stretches("s-failed", &[("2026-09-03T08:00:00Z", 7, 140)]),
            )
            .session(
                "s-old",
                "cli",
                &stretches("s-old", &[("2026-08-28T12:00:00Z", 9, 700)]),
            )
            .session(
                "s-open",
                "cli",
                &stretches("s-open", &[("2026-09-07T12:00:00Z", 1, 10)]),
            )
            .session(
                "s-never",
                "cli",
                &stretches("s-never", &[("2026-09-07T13:00:00Z", 1, 10)]),
            )
            .session(
                "s-denied",
                "cli",
                &stretches("s-denied", &[("2026-09-07T14:00:00Z", 1, 10)]),
            )
            // A human message answered without a tool call: known to have
            // no hands-off stretch.
            .session(
                "s-quiet",
                "cli",
                &[
                    human("s-quiet-h", ts("2026-09-05T15:00:00Z")),
                    plain("s-quiet-a", ts("2026-09-05T15:04:00Z")),
                ],
            )
            .session(
                "s-covered",
                "cli",
                &stretches("s-covered", &[("2026-09-04T15:00:00Z", 4, 30)]),
            );
        // 52 members of one pull request, two to a native start, among as
        // many unlinked sessions that share those starts.
        for index in 0..52_i64 {
            let at = ts("2026-09-02T00:00:00Z") + SignedDuration::from_mins(10 * (index / 2));
            let id = format!("bulk-{index:02}");
            let rows = stretches(&id, &[(&at.to_string(), 1 + index % 3, 10)]);
            seed.session(&id, "cli", &rows)
                .session(&format!("noise-{index:02}"), "cli", &rows[..1])
                .link(&id, (ATLAS, 104), PrConfidence::Exact);
        }
        seed.link("s-alpha", (ATLAS, 101), PrConfidence::Exact)
            .link("s-shared", (ATLAS, 101), PrConfidence::Exact)
            .link("s-beta", (ATLAS, 101), PrConfidence::Sha)
            .link("s-judge", (ATLAS, 101), PrConfidence::Exact)
            .link("s-shared", (ATLAS, 102), PrConfidence::Exact)
            .link("s-idle", (ATLAS, 102), PrConfidence::Inferred)
            .link("s-desk", (ATLAS, 103), PrConfidence::Exact)
            .link("s-shared", (ATLAS, 104), PrConfidence::Sha)
            .link("s-zero", (ATLAS, 105), PrConfidence::Exact)
            .link("s-beta", (ATLAS, 103), PrConfidence::Inferred)
            .link("s-old", (ATLAS, 106), PrConfidence::Exact)
            .link("s-open", (ATLAS, 107), PrConfidence::Inferred)
            .link("s-never", (ATLAS, 108), PrConfidence::Exact)
            .link("s-tools", (HARBOR, 7), PrConfidence::Exact)
            .link("s-failed", (HARBOR, 8), PrConfidence::Exact)
            .link("s-unclassified", (HARBOR, 9), PrConfidence::Sha)
            .link("s-denied", (HARBOR, 10), PrConfidence::Exact)
            // An excluded member beside a measured member without stretches.
            .link("s-desk", (ATLAS, 109), PrConfidence::Exact)
            .link("s-quiet", (ATLAS, 109), PrConfidence::Exact)
            // One type whose hands-off median is published over one of its
            // two pull requests: the other has no stretch, and none is unknown.
            .link("s-covered", (ATLAS, 110), PrConfidence::Exact)
            .link("s-quiet", (ATLAS, 111), PrConfidence::Exact)
            .merged(
                (ATLAS, 101),
                "feat: pin the drilldown to the report window",
                "feat/pinned-drilldown",
                "2026-09-06T18:00:00Z",
            )
            .merged(
                (ATLAS, 102),
                "fix: keep idle linked sessions unknown",
                "fix/idle-members",
                "2026-09-05T20:00:00Z",
            )
            .merged(
                (ATLAS, 103),
                "feat: desktop surface capture",
                "feat/desktop-capture",
                "2026-09-06T08:00:00Z",
            )
            .merged(
                (ATLAS, 104),
                "refactor: split the session page assembler across the list and the drilldown",
                "refactor/session-assembler",
                "2026-09-02T16:00:00Z",
            )
            .merged(
                (ATLAS, 105),
                "perf: cache the window bounds",
                "perf/window-bounds",
                "2026-09-07T09:00:00Z",
            )
            .merged(
                (ATLAS, 106),
                "feat: an older merge",
                "feat/older",
                "2026-08-28T15:00:00Z",
            )
            .facts(
                (ATLAS, 107),
                "feat: still open",
                "feat/open",
                PrState::Open,
                None,
            )
            .merged(
                (HARBOR, 7),
                "docs: explain overlapping effort",
                "docs/overlap",
                "2026-09-04T12:00:00Z",
            )
            .merged(
                (HARBOR, 8),
                "chore: rotate synthetic keys",
                "chore/rotate",
                "2026-09-03T09:00:00Z",
            )
            .failed((HARBOR, 8), PrRefreshError::RateLimited)
            .merged((HARBOR, 9), "untitled", "misc", "2026-09-03T12:00:00Z")
            .failed((HARBOR, 10), PrRefreshError::Unauthorized)
            .merged(
                (ATLAS, 109),
                "fix: desktop beside a quiet session",
                "fix/quiet-desktop",
                "2026-09-05T18:00:00Z",
            )
            .merged(
                (ATLAS, 110),
                "test: cover the pinned window",
                "test/pinned-window",
                "2026-09-04T18:00:00Z",
            )
            .merged(
                (ATLAS, 111),
                "test: a quiet check",
                "test/quiet",
                "2026-09-05T19:00:00Z",
            );
        drop(seed);
        raw(
            path,
            "UPDATE sessions SET kind='judge' WHERE session_id='s-judge';
             UPDATE records SET is_human=NULL WHERE uuid='s-unclassified-h0';
             UPDATE usage SET input_tokens=0 WHERE uuid='s-zero-t0';
             UPDATE pull_requests SET title=NULL
               WHERE url='https://github.com/example/harbor/pull/9';",
        );
        [
            (ATLAS, 101),
            (ATLAS, 102),
            (ATLAS, 103),
            (ATLAS, 104),
            (ATLAS, 105),
            (ATLAS, 106),
            (ATLAS, 109),
            (ATLAS, 110),
            (ATLAS, 111),
            (HARBOR, 7),
            (HARBOR, 8),
            (HARBOR, 9),
        ]
        .map(|(repository, number)| (repository.to_owned(), number))
        .into()
    }

    /// No merged pull request is known inside 7 or 14 days while some cached
    /// facts are unknown, and the trailing-14-day token gate fails: one merge
    /// only the 30-day window holds, an open pull request, a never-refreshed
    /// one and one stored as merged without a merge instant.
    fn sparse(path: &Path) -> Vec<(String, u64)> {
        let mut seed = Seed {
            store: Store::open(path).unwrap(),
        };
        let unpriced = |id: &str, at: &str| {
            vec![
                human(&format!("{id}-h"), ts(at)),
                tool(
                    &format!("{id}-t"),
                    ts(at) + SignedDuration::from_mins(2),
                    30,
                    false,
                ),
            ]
        };
        seed.session(
            "s-late",
            "cli",
            &stretches("s-late", &[("2026-08-20T10:00:00Z", 4, 500)]),
        )
        .session(
            "s-recent",
            "cli",
            &unpriced("s-recent", "2026-09-06T10:00:00Z"),
        )
        .session(
            "s-review",
            "cli",
            &unpriced("s-review", "2026-09-06T11:00:00Z"),
        )
        .link("s-late", (HARBOR, 20), PrConfidence::Exact)
        .link("s-recent", (ATLAS, 21), PrConfidence::Exact)
        .link("s-review", (HARBOR, 22), PrConfidence::Sha)
        .link("s-review", (ATLAS, 23), PrConfidence::Inferred)
        .merged(
            (HARBOR, 20),
            "fix: an earlier merge",
            "fix/earlier",
            "2026-08-20T18:00:00Z",
        )
        .facts(
            (HARBOR, 22),
            "feat: in review",
            "feat/review",
            PrState::Open,
            None,
        )
        .merged(
            (ATLAS, 23),
            "feat: merged without an instant",
            "feat/instant",
            "2026-09-06T12:00:00Z",
        );
        drop(seed);
        raw(
            path,
            "UPDATE pull_requests SET merged_at=NULL
               WHERE url='https://github.com/example/atlas/pull/23';",
        );
        vec![(HARBOR.to_owned(), 20)]
    }

    /// A drilldown page as the native command returned it, with the cursor
    /// that requested it (`null` for the first page).
    #[derive(Serialize)]
    struct Page {
        after: Option<String>,
        #[serde(flatten)]
        page: FixturePrSessions,
    }

    #[derive(Serialize)]
    struct Scenario {
        pr_analytics: Vec<PrAnalyticsPage>,
        pr_sessions: Vec<Page>,
    }

    #[derive(Serialize)]
    struct Export {
        measured: Scenario,
        sparse: Scenario,
    }

    /// The reports for every preset and mode, and every page of each named
    /// pull request's drilldown for each of them, through the commands' own
    /// assemblers at the pinned instant.
    fn scenario(seed: fn(&Path) -> Vec<(String, u64)>) -> Scenario {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("xtrace.db");
        let drilled = seed(&path);
        let now = ts(NOW).as_millisecond();
        let pr_analytics = fixture_pages(&path, now).unwrap();
        let metrics = xt_metrics::MetricsDb::open(&path).unwrap();
        let mut pr_sessions = Vec::new();
        for (repository, number) in &drilled {
            for report in &pr_analytics {
                let mut after: Option<String> = None;
                loop {
                    let request = validate(PrSessionsRequest {
                        repository,
                        number: *number,
                        confirmed_only: report.report.confirmed_only,
                        window_days: report.window.days,
                        window_end_ms: report.window.end_ms,
                        after: after.as_deref(),
                    })
                    .unwrap();
                    let page =
                        sessions(&metrics, &request, TimeZone::UTC, MetricClock::Fixture).unwrap();
                    assert_eq!(page.window, report.window);
                    let next = page.next.clone();
                    pr_sessions.push(Page {
                        after,
                        page: FixturePrSessions {
                            repository: repository.clone(),
                            number: *number,
                            confirmed_only: report.report.confirmed_only,
                            window_days: report.window.days,
                            window_end_ms: report.window.end_ms,
                            page,
                        },
                    });
                    match next {
                        Some(next) => after = Some(next),
                        None => break,
                    }
                }
            }
        }
        Scenario {
            pr_analytics,
            pr_sessions,
        }
    }

    /// The committed export is exactly what the commands produce now.
    /// Regenerate it with `XTRACE_WRITE_PR_SYNTHETIC=1`.
    #[test]
    fn the_synthetic_ui_export_is_what_the_commands_assemble() {
        let export = Export {
            measured: scenario(measured),
            sparse: scenario(sparse),
        };
        let json = serde_json::to_string_pretty(&export).unwrap() + "\n";
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(EXPORT);
        if std::env::var_os("XTRACE_WRITE_PR_SYNTHETIC").is_some() {
            std::fs::write(&path, &json).unwrap();
        }
        assert_eq!(
            json,
            std::fs::read_to_string(&path).unwrap(),
            "regenerate with XTRACE_WRITE_PR_SYNTHETIC=1"
        );

        // What the UI acceptance relies on, stated here so a changed seed
        // cannot quietly remove a case.
        let measured = &export.measured.pr_analytics[0];
        assert_eq!(
            (measured.window.days, measured.report.confirmed_only),
            (7, false)
        );
        let rows = &measured.report.rows;
        assert_eq!(rows.len(), 11);
        let find = |repository: &str, number: u64| {
            rows.iter()
                .find(|row| row.repository == repository && row.number == number)
                .unwrap()
        };
        assert_eq!(find(ATLAS, 102).tokens.counters.total_tokens, None);
        assert_eq!(find(ATLAS, 101).unmeasured_links, 1);
        assert_eq!(find(HARBOR, 7).human_messages, Some(0));
        assert_eq!(find(ATLAS, 105).tokens.counters.total_tokens, Some(0));
        assert_eq!(find(HARBOR, 9).human_messages, None);
        assert_eq!(find(HARBOR, 9).work_type, None);
        assert_eq!(find(ATLAS, 103).hands_off.excluded_sessions, 1);
        assert_eq!(find(HARBOR, 7).hands_off.n, Some(0));
        assert_eq!(find(ATLAS, 104).linked_sessions, 53);
        // Excluded plus measured without stretches: no value, not an absence.
        let mixed = &find(ATLAS, 109).hands_off;
        assert_eq!(
            (
                mixed.n,
                mixed.median_min,
                mixed.measured_sessions,
                mixed.excluded_sessions
            ),
            (Some(0), None, 1, 1)
        );
        // A published per-type median beside a no-sample pull request, with
        // a different sample for messages in the same type.
        let tests = &measured
            .report
            .by_type
            .iter()
            .find(|group| group.work_type.as_deref() == Some("test"))
            .unwrap()
            .summary;
        assert_eq!(
            (
                tests.hands_off_min.eligible_prs,
                tests.hands_off_min.measured_prs,
                tests.hands_off_min.no_sample_prs,
                tests.hands_off_min.unknown_prs
            ),
            (2, 1, 1, 0)
        );
        assert!(tests.hands_off_min.median.is_some());
        assert_eq!(tests.human_messages.measured_prs, 2);
        assert_eq!(measured.report.token_gate.passes, Some(true));
        let summary = &measured.report.summary;
        assert!(summary.human_messages.unknown_prs > 0);
        assert!(summary.tokens.median.unknown_prs > 0);
        assert!(summary.hands_off_min.no_sample_prs > 0);
        assert_ne!(
            summary.human_messages.measured_prs,
            summary.hands_off_min.measured_prs
        );
        assert_eq!(measured.report.eligibility.unknown_facts, 2);
        let sparse = &export.sparse.pr_analytics[0];
        assert!(sparse.report.rows.is_empty());
        assert_eq!(sparse.report.eligibility.unknown_facts, 2);
        assert_eq!(sparse.report.token_gate.passes, Some(false));
        assert_eq!(export.sparse.pr_analytics[4].report.rows.len(), 1);
        assert!(
            export
                .measured
                .pr_sessions
                .iter()
                .any(|page| page.after.is_some())
        );
    }
}

/// Test-only native acceptance: `seeded`'s cohort retained in a coordinator
/// prepared disposable copy of a metadata-only index, so the native PRs page
/// has populated rows and a paginated drawer, plus a Store-API adjunct for the
/// two states `seeded` lacks: a measured-zero token row and a member on a
/// surface M-09 timestamp health excludes. Never runs by default. Refuses to
/// run without `HOME` and the live index's regular-file metadata, any root but
/// a direct child of `/private/tmp/xtrace-pr-analytics-native` holding exactly
/// `data/xtrace.db` and an empty `home`, a copy that is the live index, and any
/// database that already holds a synthetic ID. Prints structural counts and
/// synthetic IDs only. Run with `XTRACE_PR_NATIVE_FIXTURE_ROOT=<root> cargo
/// test --test pr_analytics native_fixture -- --ignored --nocapture`.
#[cfg(unix)]
mod native_fixture {
    use super::*;
    use rusqlite::{Connection, OpenFlags};
    use std::ffi::OsString;
    use std::fs::Metadata;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Component, PathBuf};
    use std::time::Instant;
    use xtrace_desktop::dto::MetricSessionHandsOff;

    const PARENT: &str = "/private/tmp/xtrace-pr-analytics-native";
    const ROOT: &str = "XTRACE_PR_NATIVE_FIXTURE_ROOT";
    const LIVE: &str = "Library/Application Support/ai.xtrace.desktop/xtrace.db";

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// The live index's metadata under `home`, or a panic: without it the copy
    /// cannot be shown to be a different file.
    fn live_index(home: Option<OsString>) -> Metadata {
        let home = PathBuf::from(home.expect("HOME, to locate the live index"));
        assert!(home.is_absolute(), "an absolute HOME");
        let live = std::fs::metadata(home.join(LIVE)).expect("the live index's metadata");
        assert!(live.is_file(), "the live index is a regular file");
        live
    }

    fn not_live(db: &Metadata, live: &Metadata) {
        assert_ne!(
            (db.dev(), db.ino()),
            (live.dev(), live.ino()),
            "not the live index"
        );
    }

    /// The supplied root, or a panic before anything is opened.
    fn disposable_root() -> PathBuf {
        let live = live_index(std::env::var_os("HOME"));
        let root = PathBuf::from(std::env::var_os(ROOT).expect("an explicit disposable root"));
        assert!(root.is_absolute(), "absolute root");
        assert_eq!(root.parent(), Some(Path::new(PARENT)), "direct child");
        assert!(matches!(
            root.components().next_back(),
            Some(Component::Normal(_))
        ));
        // No alias on the way: the resolved root is the root as given.
        assert_eq!(std::fs::canonicalize(&root).unwrap(), root, "no alias");
        assert_eq!(names(&root), ["data", "home"], "only data and home");
        assert_eq!(names(&root.join("data")), ["xtrace.db"], "only the copy");
        assert!(names(&root.join("home")).is_empty(), "empty native home");
        for dir in [root.clone(), root.join("data"), root.join("home")] {
            assert!(std::fs::symlink_metadata(&dir).unwrap().is_dir());
        }
        let db = std::fs::symlink_metadata(root.join("data/xtrace.db")).unwrap();
        assert!(db.is_file(), "a regular database file");
        assert_eq!(db.nlink(), 1, "no hard-linked alias");
        not_live(&db, &live);
        root
    }

    fn refused(check: impl FnOnce() + std::panic::UnwindSafe) -> String {
        let panic = std::panic::catch_unwind(check).unwrap_err();
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
            .unwrap()
    }

    /// The live-index half of the guard, over temporary homes only: no HOME,
    /// a relative one, no live index, a live path that is not a regular file
    /// and a copy that is the live file all refuse; a distinct file passes.
    #[test]
    fn the_guard_requires_the_live_index_and_a_different_file() {
        let without = |home: Option<OsString>| {
            refused(move || {
                live_index(home);
            })
        };
        let home = tempfile::tempdir().unwrap();
        assert!(without(None).contains("HOME"));
        assert!(without(Some("relative".into())).contains("absolute"));
        assert!(without(Some(home.path().into())).contains("metadata"));
        let live = home.path().join(LIVE);
        std::fs::create_dir_all(&live).unwrap();
        assert!(without(Some(home.path().into())).contains("regular"));
        std::fs::remove_dir(&live).unwrap();
        std::fs::write(&live, b"").unwrap();
        let found = live_index(Some(home.path().into()));
        let same = std::fs::symlink_metadata(&live).unwrap();
        assert!(refused(|| not_live(&same, &found)).contains("not the live index"));
        let other = home.path().join("copy.db");
        std::fs::write(&other, b"").unwrap();
        not_live(&std::fs::symlink_metadata(&other).unwrap(), &found);
    }

    fn read_only(db: &Path) -> Connection {
        Connection::open_with_flags(
            db,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap()
    }

    const COUNTS: [(&str, &str); 12] = [
        ("schema_versions", "SELECT count(*) FROM schema_version"),
        ("schema_max", "SELECT max(version) FROM schema_version"),
        ("sessions", "SELECT count(*) FROM sessions"),
        ("records", "SELECT count(*) FROM records"),
        ("usage", "SELECT count(*) FROM usage"),
        ("tool_uses", "SELECT count(*) FROM tool_uses"),
        ("pr_links", "SELECT count(*) FROM pr_links"),
        ("pull_requests", "SELECT count(*) FROM pull_requests"),
        (
            "relations",
            "SELECT count(*) FROM session_creation_relations",
        ),
        (
            "launch_owners",
            "SELECT count(*) FROM cli_artifact_launch_owners",
        ),
        (
            "content",
            "SELECT count(*) FROM records WHERE content_json IS NOT NULL",
        ),
        (
            "confirmed",
            "SELECT count(*) FROM confirmed_automated_inputs",
        ),
    ];

    /// Structural counts, after an integrity check.
    fn counts(connection: &Connection) -> Vec<i64> {
        let integrity: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        COUNTS
            .iter()
            .map(|(_, sql)| connection.query_row(sql, [], |row| row.get(0)).unwrap())
            .collect()
    }

    /// Any row `seeded` or [`adjunct`] would write or merge into, and any
    /// session already on the adjunct's surface, whose health it would change.
    const COLLISIONS: &str = "SELECT
        (SELECT count(*) FROM sessions WHERE session_id IN
            ('s-active','s-shared','s-other','s-idle',
             'zero-member','m09-member','m09-peer-a','m09-peer-b')
            OR session_id GLOB 'bulk-*' OR session_id GLOB 'noise-*'
            OR surface = 'synthetic-m09')
      + (SELECT count(*) FROM records WHERE uuid GLOB 's-active-*'
            OR uuid GLOB 's-shared-*' OR uuid GLOB 's-other-*'
            OR uuid GLOB 'zero-member-*' OR uuid GLOB 'm09-*')
      + (SELECT count(*) FROM pull_requests
            WHERE lower(repo) IN ('example/atlas','other/atlas')
            OR lower(url) GLOB 'https://github.com/example/atlas/*'
            OR lower(url) GLOB 'https://github.com/other/atlas/*')";

    const ZERO: (&str, u64) = ("example/atlas", 6);
    const EXCLUDED: (&str, u64) = ("example/atlas", 7);
    const ZERO_MEMBER: &str = "zero-member";
    const SURFACE: &str = "synthetic-m09";
    /// The excluded row's member, then its two unlinked surface peers.
    const DEGENERATE: [&str; 3] = ["m09-member", "m09-peer-a", "m09-peer-b"];

    /// Two more merged pull requests beside `seeded`'s, through the Store API:
    /// `example/atlas#6`, whose one member's one response observes all four
    /// usage counters as zero, and `example/atlas#7`, whose one member shares
    /// a raw surface with two unlinked peers. Each of the three has six
    /// user/assistant records at one instant, so all three qualify and are
    /// degenerate and M-09 excludes the surface.
    fn adjunct(db: &Path, now: Timestamp) {
        let yesterday = now - SignedDuration::from_hours(24);
        let mut store = Store::open(db).unwrap();
        let at = yesterday + SignedDuration::from_mins(90);
        let zero: CanonicalRecord = serde_json::from_value(json!({
            "uuid":"zero-member-a","type":"assistant","timestamp":(at + SignedDuration::from_mins(2)).to_string(),
            "message":{"role":"assistant","model":"test-model",
                "content":[{"type":"text","text":"Synthetic"}],
                "usage":{"input_tokens":0,"output_tokens":0,
                         "cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}))
        .unwrap();
        session(&mut store, ZERO_MEMBER, Some(at.as_millisecond()));
        store
            .upsert_records(
                ZERO_MEMBER,
                &[record("zero-member-h", &at.to_string(), true), zero],
                false,
            )
            .unwrap();
        link(&mut store, ZERO_MEMBER, ZERO.0, ZERO.1, PrConfidence::Exact);

        let at = yesterday + SignedDuration::from_mins(120);
        for id in DEGENERATE {
            let mut meta = SessionMeta::new(id, "claude", SessionSource::Fixture);
            meta.surface = Some(SURFACE.into());
            meta.started_at_ms = Some(at.as_millisecond());
            store.upsert_session(&meta, false).unwrap();
            let rows: Vec<CanonicalRecord> = (0..6)
                .map(|index| record(&format!("{id}-{index}"), &at.to_string(), index % 2 == 0))
                .collect();
            store.upsert_records(id, &rows, false).unwrap();
        }
        link(
            &mut store,
            DEGENERATE[0],
            EXCLUDED.0,
            EXCLUDED.1,
            PrConfidence::Exact,
        );
        let merged = Some(yesterday.to_string());
        refresh(
            &mut store,
            ZERO.0,
            ZERO.1,
            "perf: measured zero",
            merged.clone(),
        );
        refresh(
            &mut store,
            EXCLUDED.0,
            EXCLUDED.1,
            "fix: excluded surface",
            merged,
        );
    }

    fn excluded_surface() -> serde_json::Value {
        json!({"host":"claude","surface":SURFACE,
               "qualifying_sessions":3,"degenerate_sessions":3})
    }

    fn timed<T>(read: impl FnOnce() -> T) -> (T, u128) {
        let start = Instant::now();
        let value = read();
        (value, start.elapsed().as_millis())
    }

    #[test]
    #[ignore = "writes the explicit disposable root in XTRACE_PR_NATIVE_FIXTURE_ROOT"]
    fn native_fixture_retains_the_seeded_cohort_in_a_disposable_index_copy() {
        retain(&disposable_root());
    }

    /// Seed `root`'s existing database and check what the app reads back.
    fn retain(root: &Path) {
        let db = root.join("data/xtrace.db");
        let before = {
            let connection = read_only(&db);
            let collisions: i64 = connection
                .query_row(COLLISIONS, [], |row| row.get(0))
                .unwrap();
            assert_eq!(collisions, 0, "synthetic ID collision");
            counts(&connection)
        };
        // A copy taken before or after this build's first open.
        assert!(
            matches!(
                (before[0], before[1]),
                (11, 11) | (12, 12) | (13, 13) | (14, 14) | (15, 15) | (16, 16) | (17, 17)
            ),
            "schema 11 to 17"
        );
        assert_eq!((before[10], before[11]), (0, 0), "content and confirmed");

        let now = Timestamp::now();
        adjunct(&db, now);
        let state = seeded(root, now);
        let metrics = xt_metrics::MetricsDb::open(&db).unwrap();
        for days in [7, 14, 30] {
            for confirmed_only in [false, true] {
                let (page, report_ms) = timed(|| state.pr_analytics(days, confirmed_only).unwrap());
                let core = metrics
                    .pr_analytics(
                        xt_metrics::Window::new(page.window.start_ms, page.window.end_ms).unwrap(),
                        page.report.now_ms,
                        confirmed_only,
                    )
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(&page.report).unwrap(),
                    serde_json::to_value(&core).unwrap()
                );
                let counts = |repository, number| {
                    row(&page, repository, number)
                        .map(|row| (row.linked_sessions, row.active_sessions))
                };
                let one = row(&page, "example/atlas", 1).unwrap();
                if confirmed_only {
                    assert_eq!(counts("example/atlas", 1), Some((2, 2)));
                    assert_eq!(one.tokens.counters.total_tokens, Some(36));
                } else {
                    assert_eq!(counts("example/atlas", 1), Some((3, 2)));
                    assert_eq!(one.tokens.counters.total_tokens, None);
                    assert_eq!(one.tokens.no_selected_usage_sessions, 1);
                    assert_eq!(
                        (one.evidence.exact, one.evidence.sha, one.evidence.inferred),
                        (1, 1, 1)
                    );
                }
                assert_eq!(counts("example/atlas", 2), Some((57, 1)));
                assert_eq!(counts("other/atlas", 1), Some((1, 1)));
                assert_eq!(counts("example/atlas", 4), None);
                assert_eq!(counts("example/atlas", 5), None);

                // The adjunct: measured zero is a known zero, not unknown.
                let zero = row(&page, ZERO.0, ZERO.1).unwrap();
                assert_eq!((zero.linked_sessions, zero.active_sessions), (1, 1));
                let counters = &zero.tokens.counters;
                assert_eq!(
                    [
                        counters.input_tokens,
                        counters.output_tokens,
                        counters.cache_read_tokens,
                        counters.cache_creation_tokens,
                        counters.total_tokens
                    ],
                    [Some(0); 5]
                );
                assert_eq!(
                    (
                        zero.tokens.measured_sessions,
                        zero.tokens.no_selected_usage_sessions,
                        zero.tokens.incomplete_sessions
                    ),
                    (1, 0, 0)
                );
                // The excluded member contributes no stretch and is named.
                let excluded = row(&page, EXCLUDED.0, EXCLUDED.1).unwrap();
                assert_eq!((excluded.linked_sessions, excluded.active_sessions), (1, 1));
                let hands_off = &excluded.hands_off;
                assert_eq!(
                    (
                        hands_off.excluded_sessions,
                        hands_off.measured_sessions,
                        hands_off.unknown_sessions,
                        hands_off.n,
                        hands_off.median_min
                    ),
                    (1, 0, 0, Some(0), None)
                );
                assert_eq!(
                    serde_json::to_value(&hands_off.excluded_surfaces).unwrap(),
                    json!([excluded_surface()])
                );
                let synthetic: Vec<serde_json::Value> = page
                    .report
                    .hands_off_excluded_surfaces
                    .iter()
                    .map(|surface| serde_json::to_value(surface).unwrap())
                    .filter(|surface| surface["surface"] == SURFACE)
                    .collect();
                assert_eq!(synthetic, [excluded_surface()]);

                let eligibility = &page.report.eligibility;
                assert_eq!(page.report.rows.len(), 5);
                assert_eq!(eligibility.merged, 5);
                assert_eq!(eligibility.outside, u64::from(!confirmed_only));
                assert!(eligibility.unknown_facts >= 1);
                let mode = if confirmed_only { "confirmed" } else { "all" };
                println!(
                    "report {days}d {mode}: rows=5 merged=5 outside={} unknown_facts={} \
                     atlas#1={:?} atlas#2=(57,1) other#1=(1,1) atlas#6 tokens=0 \
                     atlas#7 excluded=1 surface={SURFACE}(3/3) ms={report_ms}",
                    eligibility.outside,
                    eligibility.unknown_facts,
                    counts("example/atlas", 1).unwrap()
                );

                // The drawer: exact members over the report's pinned window.
                let (pages, bulk_ms) = timed(|| all_pages(&state, &page, "example/atlas", 2));
                let sizes: Vec<usize> = pages.iter().map(|page| page.rows.len()).collect();
                assert_eq!(sizes, [50, 7]);
                let listed = ids(&pages);
                let unique: BTreeSet<&String> = listed.iter().collect();
                assert_eq!(unique.len(), 57, "every member once");
                assert!(
                    listed
                        .iter()
                        .all(|id| id.starts_with("bulk-") || id == "s-shared")
                );
                assert!(pages.iter().all(|drawer| drawer.window == page.window));
                let pages = all_pages(&state, &page, "example/atlas", 1);
                let members: BTreeSet<String> = ids(&pages).into_iter().collect();
                if confirmed_only {
                    assert_eq!(members, ["s-active", "s-shared"].map(String::from).into());
                } else {
                    assert_eq!(
                        members,
                        ["s-active", "s-idle", "s-shared"].map(String::from).into()
                    );
                    let idle = pages[0].rows.iter().find(|row| row.id == "s-idle").unwrap();
                    match &idle.metrics {
                        MetricSessionWindow::Indexed { events, tokens, .. } => {
                            assert_eq!(*events, 0);
                            assert_eq!(tokens.counters.total_tokens, None);
                        }
                        other => panic!("{other:?}"),
                    }
                }
                assert_eq!(
                    ids(&all_pages(&state, &page, "other/atlas", 1)),
                    ["s-other"]
                );
                let zero = all_pages(&state, &page, ZERO.0, ZERO.1);
                assert_eq!(ids(&zero), [ZERO_MEMBER]);
                match &zero[0].rows[0].metrics {
                    MetricSessionWindow::Indexed { tokens, .. } => {
                        assert_eq!(tokens.counters.total_tokens, Some(0));
                    }
                    other => panic!("{other:?}"),
                }
                // The member only: its unlinked surface peers are not listed.
                let excluded = all_pages(&state, &page, EXCLUDED.0, EXCLUDED.1);
                assert_eq!(ids(&excluded), [DEGENERATE[0]]);
                match &excluded[0].rows[0].hands_off {
                    MetricSessionHandsOff::Unmeasured {
                        excluded_surface: Some(surface),
                    } => assert_eq!(serde_json::to_value(surface).unwrap(), excluded_surface()),
                    other => panic!("{other:?}"),
                }
                println!(
                    "drawer {days}d {mode}: atlas#2 pages={sizes:?} unique=57 \
                     atlas#1={members:?} other#1=[\"s-other\"] atlas#6=[\"{ZERO_MEMBER}\"] \
                     atlas#7=[\"{}\"] ms={bulk_ms}",
                    DEGENERATE[0]
                );
            }
        }

        // Only the synthetic cohort was added; every original fact is kept.
        let after = counts(&read_only(&db));
        for (index, (name, _)) in COUNTS.iter().enumerate() {
            println!("{name}: {} -> {}", before[index], after[index]);
        }
        let added: Vec<i64> = before.iter().zip(&after).map(|(b, a)| a - b).collect();
        assert_eq!((after[0], after[1]), (17, 17), "schema 17");
        // `seeded`'s, then the adjunct's: one zero member and three on the
        // excluded surface, each degenerate session's three assistant records
        // with one usage row and one tool call apiece.
        assert_eq!(added[2], 4 + 56 + 55 + 1 + 3, "sessions");
        assert_eq!(added[3], 6 + 2 + 3 * 6, "records");
        assert_eq!(added[4], 3 + 1 + 3 * 3, "usage");
        assert_eq!(added[5], 3 + 3 * 3, "tool uses");
        assert_eq!(added[6], 56 + 7 + 2, "links");
        assert_eq!(added[7], 5 + 2, "pull requests");
        assert_eq!((added[8], added[9]), (0, 0), "relations");
        assert_eq!((after[10], after[11]), (0, 0), "content and confirmed");
        drop(metrics);
        state.shutdown();
    }
}
