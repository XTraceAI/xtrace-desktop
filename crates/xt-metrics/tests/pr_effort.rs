use jiff::{Timestamp, tz::TimeZone};
use serde_json::{Value, json};
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{
    COST_BASIS, CostSummary, DayEffort, EffortAssignment, EffortTotals, MetricsDb, ModelDayEffort,
    PrEffortReport, PrFreshness, PriceCatalog, Window,
};
use xt_store::{
    CanonicalRecord,
    pr_link::{
        PrConfidence, PrIdentity, PrLinkObservation, PrRefreshError, PrState, RefreshFailure,
        RefreshOutcome, RefreshSuccess,
    },
};

fn ms(text: &str) -> i64 {
    text.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn seed(db: &mut TempDb, id: &str, records: &[CanonicalRecord]) {
    let mut session =
        Fixture::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F1"))
            .unwrap()
            .sessions()[0]
            .metadata
            .clone();
    session.session_id = id.into();
    db.store_mut().upsert_session(&session, false).unwrap();
    db.store_mut().upsert_records(id, records, false).unwrap();
}
fn event(id: &str, ts: &str) -> CanonicalRecord {
    serde_json::from_value(
        json!({"uuid":id,"type":"assistant","timestamp":ts,"message":{"role":"assistant"}}),
    )
    .unwrap()
}
/// A synthetic catalog in which one output token costs exactly one dollar and
/// nothing else is charged, so a dollar total reads as the output count.
fn catalog() -> PriceCatalog {
    let rates = json!({"input":0,"output":1_000_000_000_u64,"cache_read":0,"cache_write":0,
        "cache_write_5m":null,"cache_write_1h":null});
    PriceCatalog::from_json(
        &json!({"version":"effort-test","as_of":"2026-09-17","basis":COST_BASIS,
            "rate_unit":"nano_usd_per_token","sources":["https://example.test/prices"],
            "models":[{"id":"effort-test","aliases":[],"cache_write_mode":"flat",
                "bands":[{"max_prompt_tokens":null,
                    "tiers":[{"names":["standard"],"rates":rates}]}]}]})
        .to_string(),
    )
    .unwrap()
}
/// A priced total in whole dollars; `None` when any selected response is
/// unpriced or none was selected, as the report's `total_usd` says.
fn dollars(cost: &CostSummary) -> Option<u64> {
    cost.total_usd.map(|usd| usd as u64)
}
/// One assistant response with measured input and the given output counter,
/// on the synthetic catalog's model. A missing output leaves it unpriced.
fn response(id: &str, ts: &str, output: Option<u64>) -> CanonicalRecord {
    let mut usage = json!({"input_tokens":1,"cache_read_input_tokens":0,
        "cache_creation_input_tokens":0,"service_tier":"standard"});
    if let Some(output) = output {
        usage["output_tokens"] = json!(output);
    }
    serde_json::from_value(json!({"uuid":id,"type":"assistant","timestamp":ts,
        "message":{"role":"assistant","model":"effort-test","usage":usage}}))
    .unwrap()
}
fn keyed(mut record: CanonicalRecord, message: &str) -> CanonicalRecord {
    record.api_message_id = Some(message.into());
    record.request_id = Some(format!("request-{message}"));
    record
}
fn url(number: u64) -> String {
    format!("https://github.com/xtrace/app/pull/{number}")
}
fn link(db: &mut TempDb, session: &str, number: u64, confidence: PrConfidence) {
    db.store_mut()
        .record_pr_link(&PrLinkObservation {
            session_id: session.into(),
            pull_request: PrIdentity::from_url(&url(number)).unwrap(),
            confidence,
            first_seen_at: 1,
            last_seen_at: 2,
        })
        .unwrap();
}
fn refresh(
    db: &mut TempDb,
    number: u64,
    at: i64,
    state: PrState,
    merged_at: Option<&str>,
    title: &str,
    head: &str,
) {
    db.store_mut()
        .record_pr_refresh(&RefreshOutcome::Success(RefreshSuccess {
            pull_request: PrIdentity::from_url(&url(number)).unwrap(),
            attempted_at: at,
            title: title.into(),
            state,
            merged_at: merged_at.map(Into::into),
            additions: 1,
            deletions: 1,
            head_ref_name: head.into(),
        }))
        .unwrap();
}
fn merged(db: &mut TempDb, number: u64, merged_at: &str, title: &str) {
    refresh(
        db,
        number,
        100,
        PrState::Merged,
        Some(merged_at),
        title,
        "main",
    );
}
fn fail(db: &mut TempDb, number: u64, at: i64, error: PrRefreshError) {
    db.store_mut()
        .record_pr_refresh(&RefreshOutcome::Failure(RefreshFailure {
            pull_request: PrIdentity::from_url(&url(number)).unwrap(),
            attempted_at: at,
            error,
        }))
        .unwrap();
}
fn report(db: &TempDb, confirmed_only: bool) -> PrEffortReport {
    let report = MetricsDb::open(db.path())
        .unwrap()
        .pr_effort(window(), TimeZone::UTC, confirmed_only, &catalog())
        .unwrap();
    reconcile(db, window(), TimeZone::UTC, &report);
    report
}
fn ty(name: &str) -> EffortAssignment {
    EffortAssignment::Type(name.into())
}
fn group<'a>(report: &'a PrEffortReport, assignment: &EffortAssignment) -> &'a EffortTotals {
    &report
        .by_assignment
        .iter()
        .find(|g| &g.assignment == assignment)
        .unwrap_or_else(|| panic!("no {assignment:?} in {:?}", report.by_assignment))
        .effort
}
fn assignments(report: &PrEffortReport) -> Vec<(EffortAssignment, u64)> {
    report
        .by_assignment
        .iter()
        .map(|g| (g.assignment.clone(), g.effort.sessions))
        .collect()
}

/// Assignments are disjoint and add to the ungrouped cohort, which equals the
/// existing whole-window reports: sessions, M-05 time and the cost report, per
/// day.
fn reconcile(db: &TempDb, window: Window, zone: TimeZone, report: &PrEffortReport) {
    let metrics = MetricsDb::open(db.path()).unwrap();
    let spans = metrics.active_spans(window).unwrap();
    let cost = metrics.cost(window, zone.clone(), &catalog()).unwrap();
    let cohort = &report.cohort;
    let mut sessions: Vec<_> = spans.spans.iter().map(|s| s.session_id.clone()).collect();
    sessions.dedup();
    assert_eq!(cohort.sessions, sessions.len() as u64);
    assert_eq!(cohort.agent_ms, spans.active_ms);
    assert_eq!(cohort.cost, cost.total);
    let days = spans.by_day(window, zone).unwrap();
    assert_eq!(cohort.by_day.len(), days.len());
    for (index, (day, (agent, priced))) in cohort
        .by_day
        .iter()
        .zip(days.iter().zip(&cost.by_day))
        .enumerate()
    {
        assert_eq!(day.date, agent.date);
        assert_eq!((day.start_ms, day.end_ms), (agent.start_ms, agent.end_ms));
        assert_eq!(day.agent_ms, agent.active_ms);
        assert_eq!(day.cost, priced.cost);
        let groups = report.by_assignment.iter().map(|g| &g.effort.by_day[index]);
        assert_eq!(
            groups.clone().map(|d| d.agent_ms).sum::<u64>(),
            day.agent_ms
        );
        assert_eq!(
            groups
                .clone()
                .map(|d| d.cost.selected_observations)
                .sum::<u64>(),
            day.cost.selected_observations
        );
        assert_eq!(
            groups.map(|d| d.cost.priced_subtotal_usd).sum::<f64>(),
            day.cost.priced_subtotal_usd
        );
    }
    for totals in std::iter::once(cohort).chain(report.by_assignment.iter().map(|g| &g.effort)) {
        for day in &totals.by_day {
            models_add_up(day);
        }
    }
    let groups = report.by_assignment.iter().map(|g| &g.effort);
    assert_eq!(
        groups.clone().map(|g| g.sessions).sum::<u64>(),
        cohort.sessions
    );
    assert_eq!(
        groups.clone().map(|g| g.agent_ms).sum::<u64>(),
        cohort.agent_ms
    );
    assert_eq!(
        groups
            .clone()
            .map(|g| g.cost.selected_observations)
            .sum::<u64>(),
        cohort.cost.selected_observations
    );
    assert_eq!(
        groups.map(|g| g.cost.priced_subtotal_usd).sum::<f64>(),
        cohort.cost.priced_subtotal_usd
    );
}

/// A day's models add up to the day exactly: priced nano-USD, priced and
/// unpriced counts and agent time, each model once, in name order.
fn models_add_up(day: &DayEffort) {
    let names: Vec<_> = day.models.iter().map(|m| m.model.as_deref()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(names, sorted, "{}", day.date);
    let nano: u64 = day.models.iter().map(|m| m.priced_nano_usd).sum();
    assert_eq!(nano as f64 / 1_000_000_000.0, day.cost.priced_subtotal_usd);
    assert_eq!(
        day.models
            .iter()
            .map(|m| m.priced_observations)
            .sum::<u64>(),
        day.cost.priced_observations
    );
    assert_eq!(
        day.models
            .iter()
            .map(|m| m.unpriced_observations)
            .sum::<u64>(),
        day.cost.unpriced_observations
    );
    assert_eq!(
        day.models.iter().map(|m| m.agent_ms).sum::<u64>(),
        day.agent_ms
    );
    assert!(
        day.models
            .iter()
            .all(|m| m.priced_observations + m.unpriced_observations + m.agent_ms > 0)
    );
}

/// F19: one 100k-output session linked exactly to two merged PRs of different
/// types, and a variant whose second link is inferred.
fn f19(second: PrConfidence) -> TempDb {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "shared",
        &[
            response("r1", "2026-09-03T10:00:00Z", Some(60_000)),
            response("r2", "2026-09-03T10:10:00Z", Some(40_000)),
        ],
    );
    link(&mut db, "shared", 1, PrConfidence::Exact);
    link(&mut db, "shared", 2, second);
    merged(&mut db, 1, "2026-09-04T09:00:00Z", "feat: add effort");
    merged(&mut db, 2, "2026-09-05T09:00:00Z", "ENG-12: fix overlap");
    db
}

#[test]
fn pr_effort_f19_overlapping_session_counts_once_under_mixed() {
    let db = f19(PrConfidence::Exact);
    let report = report(&db, false);
    assert_eq!(report.tile.known_merged, 2);
    assert_eq!(report.tile.merged, Some(2));
    assert!(report.tile.complete);
    assert_eq!(
        report
            .markers
            .iter()
            .map(|m| (m.number, m.date.as_str(), m.work_type.as_deref()))
            .collect::<Vec<_>>(),
        [
            (1, "2026-09-04", Some("feat")),
            (2, "2026-09-05", Some("fix"))
        ]
    );
    assert_eq!(assignments(&report), [(EffortAssignment::Mixed, 1)]);
    let mixed = group(&report, &EffortAssignment::Mixed);
    assert_eq!(dollars(&mixed.cost), Some(100_000));
    assert_eq!(dollars(&report.cohort.cost), Some(100_000));
    assert_eq!(mixed.agent_ms, 600_000);
    // Event day, not merge day.
    let day = |d: &str| mixed.by_day.iter().find(|x| x.date == d).unwrap().clone();
    assert_eq!(dollars(&day("2026-09-03").cost), Some(100_000));
    assert_eq!(dollars(&day("2026-09-04").cost), None);
    assert_eq!(day("2026-09-04").cost.selected_observations, 0);
    // confirmed_only has nothing inferred to remove here.
    assert_eq!(report.by_assignment, self::report(&db, true).by_assignment);
}

#[test]
fn pr_effort_f19_inferred_variant_is_removed_before_tile_and_type() {
    let db = f19(PrConfidence::Inferred);
    let all = report(&db, false);
    assert_eq!(all.tile.known_merged, 2);
    assert_eq!(assignments(&all), [(EffortAssignment::Mixed, 1)]);
    let confirmed = report(&db, true);
    assert!(confirmed.confirmed_only);
    assert_eq!(confirmed.tile.known_merged, 1);
    assert_eq!(confirmed.markers.len(), 1);
    assert_eq!(confirmed.markers[0].confidence, PrConfidence::Exact);
    assert_eq!(assignments(&confirmed), [(ty("feat"), 1)]);
    assert_eq!(dollars(&group(&confirmed, &ty("feat")).cost), Some(100_000));

    // An inferred link to a PR without facts leaves a session unresolved only
    // until confirmed_only removes that link first.
    let mut db = f19(PrConfidence::Exact);
    link(&mut db, "shared", 3, PrConfidence::Inferred);
    let mut single = TempDb::empty().unwrap();
    seed(
        &mut single,
        "s",
        &[response("r", "2026-09-03T10:00:00Z", Some(5))],
    );
    link(&mut single, "s", 1, PrConfidence::Sha);
    link(&mut single, "s", 3, PrConfidence::Inferred);
    merged(&mut single, 1, "2026-09-04T09:00:00Z", "feat: x");
    let all = report(&single, false);
    assert_eq!(assignments(&all), [(EffortAssignment::Unresolved, 1)]);
    assert_eq!((all.tile.unknown_facts, all.tile.merged), (1, None));
    let confirmed = report(&single, true);
    assert_eq!(assignments(&confirmed), [(ty("feat"), 1)]);
    assert_eq!(
        (confirmed.tile.unknown_facts, confirmed.tile.merged),
        (0, Some(1))
    );
    // Two known types are mixed whatever the unknown link would add.
    assert_eq!(
        assignments(&report(&db, false)),
        [(EffortAssignment::Mixed, 1)]
    );
}

#[test]
fn pr_effort_same_type_prs_assign_the_session_once() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[response("r", "2026-09-03T10:00:00Z", Some(100_000))],
    );
    link(&mut db, "s", 1, PrConfidence::Exact);
    link(&mut db, "s", 2, PrConfidence::Exact);
    merged(&mut db, 1, "2026-09-04T09:00:00Z", "feat: one");
    merged(&mut db, 2, "2026-09-05T09:00:00Z", "Feature: two");
    let report = report(&db, false);
    assert_eq!(report.tile.merged, Some(2));
    assert_eq!(assignments(&report), [(ty("feat"), 1)]);
    assert_eq!(dollars(&group(&report, &ty("feat")).cost), Some(100_000));
}

#[test]
fn pr_effort_unlinked_open_closed_outside_and_missing_cache() {
    let mut db = TempDb::empty().unwrap();
    for session in [
        "unlinked",
        "open",
        "closed",
        "before",
        "at-end",
        "never",
        "failed",
        "known-unknown",
        "merged-null",
        "untitled",
        "leap",
    ] {
        seed(
            &mut db,
            session,
            &[response(
                &format!("{session}-r"),
                "2026-09-03T10:00:00Z",
                Some(1),
            )],
        );
    }
    let mut number = 0;
    let mut pr = |db: &mut TempDb, session: &str| {
        number += 1;
        link(db, session, number, PrConfidence::Exact);
        number
    };
    let open = pr(&mut db, "open");
    refresh(
        &mut db,
        open,
        100,
        PrState::Open,
        None,
        "feat: open",
        "main",
    );
    let closed = pr(&mut db, "closed");
    refresh(
        &mut db,
        closed,
        100,
        PrState::Closed,
        None,
        "feat: closed",
        "main",
    );
    let before = pr(&mut db, "before");
    merged(&mut db, before, "2026-08-31T23:59:59.999Z", "feat: before");
    // Half-open window: the end instant is outside.
    let at_end = pr(&mut db, "at-end");
    merged(&mut db, at_end, "2026-09-08T00:00:00Z", "feat: at end");
    pr(&mut db, "never");
    let failed = pr(&mut db, "failed");
    fail(&mut db, failed, 50, PrRefreshError::Unauthorized);
    let known = pr(&mut db, "known-unknown");
    merged(&mut db, known, "2026-09-02T00:00:00Z", "docs: known");
    pr(&mut db, "known-unknown");
    // A MERGED row without its instant cannot be placed in any window.
    let merged_null = pr(&mut db, "merged-null");
    merged(&mut db, merged_null, "2026-09-02T00:00:00Z", "chore: x");
    let raw = rusqlite::Connection::open(db.path()).unwrap();
    raw.execute(
        "UPDATE pull_requests SET merged_at=NULL WHERE number=?1",
        [merged_null as i64],
    )
    .unwrap();
    // Missing required classification facts: eligible, type unresolved.
    let untitled = pr(&mut db, "untitled");
    merged(&mut db, untitled, "2026-09-02T00:00:00Z", "Update");
    raw.execute(
        "UPDATE pull_requests SET head_ref_name=NULL WHERE number=?1",
        [untitled as i64],
    )
    .unwrap();
    // A leap second before the end belongs to the window's final day.
    let leap = pr(&mut db, "leap");
    merged(&mut db, leap, "2026-09-07T23:59:60.5Z", "perf: leap");

    let report = report(&db, false);
    let unresolved = EffortAssignment::Unresolved;
    assert_eq!(
        assignments(&report),
        [
            (ty("perf"), 1),
            (EffortAssignment::Other, 5),
            (unresolved.clone(), 5)
        ]
    );
    assert_eq!(dollars(&group(&report, &unresolved).cost), Some(5));
    assert_eq!(
        dollars(&group(&report, &EffortAssignment::Other).cost),
        Some(5)
    );
    // docs, untitled, leap merged in window; never, failed, second known-unknown
    // link and merged-null have unknown facts.
    assert_eq!(report.tile.known_merged, 3);
    assert_eq!(report.tile.unknown_facts, 4);
    assert!(!report.tile.complete);
    assert_eq!(report.tile.merged, None);
    assert_eq!(report.tile.unresolved_type, 1);
    assert_eq!(
        report
            .markers
            .iter()
            .map(|m| (m.number, m.date.as_str(), m.work_type.as_deref()))
            .collect::<Vec<_>>(),
        [
            (known, "2026-09-02", Some("docs")),
            (untitled, "2026-09-02", None),
            (leap, "2026-09-07", Some("perf")),
        ]
    );
    let freshness = &report.tile.freshness;
    assert_eq!(
        (
            freshness.never_attempted,
            freshness.refreshed,
            freshness.failed_never_refreshed,
            freshness.failed_after_refresh,
        ),
        (2, 8, 1, 0)
    );
    assert_eq!(freshness.oldest_refreshed_at, Some(100));
    assert_eq!(freshness.newest_attempted_at, Some(100));
}

#[test]
fn pr_effort_classifier_aliases_and_other_share_the_type_panel() {
    let mut db = TempDb::empty().unwrap();
    for (session, title, head) in [
        ("hotfix", "hotfix: now", "main"),
        (
            "branch",
            "POR-4441/Rewards 上線推廣 UI",
            "feat/4441-rewards-launch-ui",
        ),
        ("slug", "Update", "user/eng-866-fix-the-thing"),
        ("improvement", "improvements to docs", "main"),
        ("unrecognized", "Update", "main"),
    ] {
        seed(
            &mut db,
            session,
            &[response(
                &format!("{session}-r"),
                "2026-09-03T10:00:00Z",
                Some(1),
            )],
        );
        let number = session.len() as u64 * 10 + u64::from(session.as_bytes()[0]);
        link(&mut db, session, number, PrConfidence::Exact);
        refresh(
            &mut db,
            number,
            100,
            PrState::Merged,
            Some("2026-09-04T00:00:00Z"),
            title,
            head,
        );
    }
    // An unrecognized PR is classified `other`, and a session whose only type
    // is `other` shares the bucket with sessions that have no eligible link.
    seed(&mut db, "unlinked", &[event("u", "2026-09-03T10:00:00Z")]);
    // Categories are not collapsed: `other` next to `feat` is two types.
    seed(&mut db, "both", &[event("b", "2026-09-03T10:00:00Z")]);
    link(
        &mut db,
        "both",
        6 * 10 + u64::from(b'b'),
        PrConfidence::Exact,
    );
    link(
        &mut db,
        "both",
        12 * 10 + u64::from(b'u'),
        PrConfidence::Exact,
    );
    let report = report(&db, false);
    assert_eq!(
        assignments(&report),
        [
            (ty("feat"), 1),
            (ty("fix"), 1),
            (ty("hotfix"), 1),
            (ty("improvement"), 1),
            (EffortAssignment::Mixed, 1),
            (EffortAssignment::Other, 2),
        ]
    );
    let mut types: Vec<_> = report
        .markers
        .iter()
        .map(|m| m.work_type.clone().unwrap())
        .collect();
    types.sort();
    assert_eq!(types, ["feat", "fix", "hotfix", "improvement", "other"]);
}

#[test]
fn pr_effort_global_response_selection_precedes_session_and_window_grouping() {
    let mut db = TempDb::empty().unwrap();
    // Latest snapshot is after the window: the response is not in it, although
    // an earlier snapshot is; the session is still in the cohort.
    seed(
        &mut db,
        "late",
        &[
            keyed(response("late-a", "2026-09-07T23:59:00Z", Some(10)), "late"),
            keyed(response("late-b", "2026-09-08T00:00:01Z", Some(20)), "late"),
        ],
    );
    // Latest snapshot is inside: counted once, with its own counter.
    seed(
        &mut db,
        "early",
        &[
            keyed(
                response("early-a", "2026-08-31T23:59:00Z", Some(10)),
                "early",
            ),
            keyed(
                response("early-b", "2026-09-01T00:01:00Z", Some(30)),
                "early",
            ),
        ],
    );
    // One response copied into two differently typed sessions counts once, in
    // the session holding the selected snapshot.
    seed(
        &mut db,
        "copy-a",
        &[keyed(
            response("copy-1", "2026-09-03T10:00:00Z", Some(7)),
            "copy",
        )],
    );
    seed(
        &mut db,
        "copy-b",
        &[keyed(
            response("copy-2", "2026-09-03T10:00:00Z", Some(7)),
            "copy",
        )],
    );
    for (session, number, title) in [
        ("late", 1, "feat: a"),
        ("early", 2, "fix: b"),
        ("copy-a", 3, "docs: c"),
        ("copy-b", 4, "test: d"),
    ] {
        link(&mut db, session, number, PrConfidence::Exact);
        merged(&mut db, number, "2026-09-04T00:00:00Z", title);
    }
    let report = report(&db, false);
    let late = group(&report, &ty("feat"));
    assert_eq!((late.sessions, late.cost.selected_observations), (1, 0));
    assert_eq!(dollars(&late.cost), None);
    let early = group(&report, &ty("fix"));
    assert_eq!(dollars(&early.cost), Some(30));
    assert_eq!(dollars(&group(&report, &ty("docs")).cost), None);
    assert_eq!(dollars(&group(&report, &ty("test")).cost), Some(7));
    assert_eq!(dollars(&report.cohort.cost), Some(37));
    assert_eq!(report.cohort.cost.selected_observations, 2);
}

#[test]
fn pr_effort_output_zero_unknown_and_no_observation_stay_distinct() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "zero",
        &[response("z", "2026-09-03T10:00:00Z", Some(0))],
    );
    seed(
        &mut db,
        "unknown",
        &[response("k", "2026-09-03T10:00:00Z", None)],
    );
    seed(&mut db, "none", &[event("n", "2026-09-03T10:00:00Z")]);
    for (session, number, title) in [
        ("zero", 1, "feat: z"),
        ("unknown", 2, "fix: k"),
        ("none", 3, "docs: n"),
    ] {
        link(&mut db, session, number, PrConfidence::Exact);
        merged(&mut db, number, "2026-09-04T00:00:00Z", title);
    }
    let report = report(&db, false);
    let output = |t: &str| {
        let c = &group(&report, &ty(t)).cost;
        (dollars(c), c.selected_observations, c.priced_observations)
    };
    assert_eq!(output("feat"), (Some(0), 1, 1));
    assert_eq!(output("fix"), (None, 1, 0));
    assert_eq!(output("docs"), (None, 0, 0));
    let cohort = &report.cohort.cost;
    assert_eq!(
        (
            dollars(cohort),
            cohort.selected_observations,
            cohort.priced_observations
        ),
        (None, 2, 1)
    );
    let json: Value = serde_json::to_value(&report).unwrap();
    assert_eq!(json["cohort"]["cost"]["total_usd"], Value::Null);
}

#[test]
fn pr_effort_agent_time_splits_at_dst_and_midnight_with_parallel_sessions_adding() {
    let zone = TimeZone::get("America/New_York").unwrap();
    // 2026-11-01 is 25 hours long in New York.
    let window = Window::new(ms("2026-10-31T04:00:00Z"), ms("2026-11-03T05:00:00Z")).unwrap();
    let mut db = TempDb::empty().unwrap();
    for session in ["a", "b"] {
        // 23:50, 00:00, 00:10 local on Oct 31 → Nov 1 (EDT, UTC-4).
        seed(
            &mut db,
            session,
            &[
                event(&format!("{session}1"), "2026-11-01T03:50:00Z"),
                event(&format!("{session}2"), "2026-11-01T04:00:00Z"),
                event(&format!("{session}3"), "2026-11-01T04:10:00Z"),
            ],
        );
        link(&mut db, session, 1, PrConfidence::Exact);
    }
    // 23:55 → 00:05 local on Nov 1 → Nov 2 (EST, UTC-5), unlinked.
    seed(
        &mut db,
        "c",
        &[
            event("c1", "2026-11-02T04:55:00Z"),
            event("c2", "2026-11-02T05:05:00Z"),
        ],
    );
    merged(&mut db, 1, "2026-11-01T12:00:00Z", "feat: dst");
    let report = MetricsDb::open(db.path())
        .unwrap()
        .pr_effort(window, zone.clone(), false, &catalog())
        .unwrap();
    reconcile(&db, window, zone, &report);
    let feat = group(&report, &ty("feat"));
    assert_eq!((feat.sessions, feat.agent_ms), (2, 2_400_000));
    let days: Vec<_> = feat
        .by_day
        .iter()
        .map(|d| (d.date.as_str(), d.end_ms - d.start_ms, d.agent_ms))
        .collect();
    assert_eq!(
        days,
        [
            ("2026-10-31", 86_400_000, 1_200_000),
            ("2026-11-01", 90_000_000, 1_200_000),
            ("2026-11-02", 86_400_000, 0),
        ]
    );
    let other = group(&report, &EffortAssignment::Other);
    assert_eq!(
        other.by_day.iter().map(|d| d.agent_ms).collect::<Vec<_>>(),
        [0, 300_000, 300_000]
    );
    assert_eq!(report.markers[0].date, "2026-11-01");
}

#[test]
fn pr_effort_failed_refresh_keeps_cached_success_with_its_status() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[response("r", "2026-09-03T10:00:00Z", Some(9))],
    );
    link(&mut db, "s", 1, PrConfidence::Exact);
    merged(&mut db, 1, "2026-09-04T00:00:00Z", "feat: cached");
    fail(&mut db, 1, 200, PrRefreshError::RateLimited);
    let before = db.store().counts().unwrap();
    let report = report(&db, false);
    assert_eq!(db.store().counts().unwrap(), before);
    assert_eq!(report.tile.merged, Some(1));
    assert_eq!(
        report.markers[0].freshness,
        PrFreshness::FailedAfterRefresh(PrRefreshError::RateLimited)
    );
    let freshness = &report.tile.freshness;
    assert_eq!(freshness.failed_after_refresh, 1);
    assert_eq!(
        (freshness.oldest_refreshed_at, freshness.newest_attempted_at),
        (Some(100), Some(200))
    );
    assert_eq!(assignments(&report), [(ty("feat"), 1)]);
    assert_eq!(
        serde_json::to_value(report.markers[0].freshness).unwrap(),
        json!({"state":"failed_after_refresh","error":"rate_limited"})
    );
}

#[test]
fn pr_effort_tile_counts_retained_links_outside_the_cohort() {
    let mut db = TempDb::empty().unwrap();
    // The linked session has no in-window work; the PR still counts.
    seed(&mut db, "old", &[event("o", "2026-08-01T00:00:00Z")]);
    link(&mut db, "old", 1, PrConfidence::Exact);
    merged(&mut db, 1, "2026-09-04T00:00:00Z", "feat: x");
    let report = report(&db, false);
    assert_eq!(report.tile.merged, Some(1));
    assert_eq!(report.cohort.sessions, 0);
    assert!(report.by_assignment.is_empty());
    assert!(report.cohort.by_day.iter().all(|d| d.agent_ms == 0
        && dollars(&d.cost).is_none()
        && d.cost.selected_observations == 0));
}

/// A response on `model`, with the four canonical counters, an optional
/// recorded tier and an optional Claude five-minute/one-hour write split.
fn priced(
    id: &str,
    ts: &str,
    model: &str,
    tier: Option<&str>,
    [input, output, read, write]: [u64; 4],
    split: Option<[u64; 2]>,
) -> CanonicalRecord {
    let mut usage = json!({"input_tokens":input,"output_tokens":output,
        "cache_read_input_tokens":read,"cache_creation_input_tokens":write,"service_tier":tier});
    if let Some([five, hour]) = split {
        usage["cache_creation"] =
            json!({"ephemeral_5m_input_tokens":five,"ephemeral_1h_input_tokens":hour});
    }
    serde_json::from_value(json!({"uuid":id,"type":"assistant","timestamp":ts,
        "message":{"role":"assistant","model":model,"usage":usage}}))
    .unwrap()
}

#[test]
fn pr_effort_dollars_per_assignment_and_day_price_each_response_by_its_model() {
    let mut db = TempDb::empty().unwrap();
    for (id, host) in [("claude-s", "claude"), ("codex-s", "codex")] {
        let session = xt_store::SessionMeta::new(id, host, xt_store::SessionSource::Fixture);
        db.store_mut().upsert_session(&session, false).unwrap();
    }
    db.store_mut()
        .upsert_records(
            "claude-s",
            &[
                priced(
                    "c1",
                    "2026-09-03T10:00:00Z",
                    "claude-opus-5-5",
                    Some("standard"),
                    [1000, 100, 2000, 500],
                    Some([300, 200]),
                ),
                priced(
                    "c2",
                    "2026-09-04T10:00:00Z",
                    "claude-fable-5",
                    Some("standard"),
                    [0, 1000, 0, 0],
                    None,
                ),
            ],
            false,
        )
        .unwrap();
    db.store_mut()
        .upsert_records(
            "codex-s",
            &[
                priced(
                    "x1",
                    "2026-09-03T11:00:00Z",
                    "gpt-6-sol",
                    None,
                    [1000, 100, 2000, 500],
                    None,
                ),
                priced(
                    "x2",
                    "2026-09-03T11:05:00Z",
                    "codex-auto-review",
                    None,
                    [10, 10, 0, 0],
                    None,
                ),
                priced(
                    "x3",
                    "2026-09-04T11:00:00Z",
                    "gpt-5.5",
                    None,
                    [1000, 0, 0, 0],
                    None,
                ),
            ],
            false,
        )
        .unwrap();
    link(&mut db, "claude-s", 1, PrConfidence::Exact);
    merged(&mut db, 1, "2026-09-05T09:00:00Z", "feat: priced");
    let prices = PriceCatalog::bundled().unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let report = metrics
        .pr_effort(window(), TimeZone::UTC, false, &prices)
        .unwrap();
    let day = |totals: &EffortTotals, date: &str| {
        totals
            .by_day
            .iter()
            .find(|d| d.date == date)
            .unwrap()
            .cost
            .clone()
    };

    // Claude: 5m and 1h writes at their own rates, then a Fable day.
    let feat = group(&report, &ty("feat"));
    assert_eq!(day(feat, "2026-09-03").total_usd, Some(0.0095));
    assert_eq!(day(feat, "2026-09-04").total_usd, Some(0.05));
    assert_eq!(feat.cost.total_usd, Some(0.0595));
    assert_eq!(feat.cost.assumed_tier_observations, 0);

    // Codex: no recorded tier, priced at the default tier and counted as
    // assumed; the unpublished review model is named, never added as zero.
    let other = group(&report, &EffortAssignment::Other);
    let first = day(other, "2026-09-03");
    assert_eq!(first.total_usd, None);
    assert_eq!(first.priced_subtotal_usd, 0.00465);
    assert_eq!(
        (
            first.selected_observations,
            first.priced_observations,
            first.unpriced_observations,
            first.assumed_tier_observations
        ),
        (2, 1, 1, 1)
    );
    assert_eq!(
        first.unpriced[0].model.as_deref(),
        Some("codex-auto-review")
    );
    assert_eq!(
        first.unpriced[0].reason,
        xt_metrics::UnpricedReason::UnknownModel
    );
    assert_eq!(day(other, "2026-09-04").total_usd, Some(0.005));
    assert_eq!(other.cost.priced_subtotal_usd, 0.00965);
    assert_eq!(other.cost.assumed_tier_observations, 2);

    // The cohort is exactly the Dashboard's cost report, total and per day.
    let cost = metrics.cost(window(), TimeZone::UTC, &prices).unwrap();
    assert_eq!(report.cohort.cost, cost.total);
    assert_eq!(report.cohort.cost.priced_subtotal_usd, 0.06915);
    for (effort, priced) in report.cohort.by_day.iter().zip(&cost.by_day) {
        assert_eq!(effort.cost, priced.cost);
    }
}

fn model_response(id: &str, ts: &str, model: &str, output: u64) -> CanonicalRecord {
    let mut record = response(id, ts, Some(output));
    let mut value = serde_json::to_value(&record).unwrap();
    value["message"]["model"] = json!(model);
    record = serde_json::from_value(value).unwrap();
    record
}

/// Cost goes to each response's own model; agent time goes, whole, to the
/// model the session used most over the window (most responses, then most
/// output tokens, then the first name), and to no model when the session has
/// no response that names one.
#[test]
fn pr_effort_days_split_by_model_cost_by_response_hours_by_most_used_model() {
    let mut db = TempDb::empty().unwrap();
    let day = |time: &str| format!("2026-09-02T{time}:00Z");
    // More responses win, although the other model wrote more output.
    seed(
        &mut db,
        "a",
        &[
            model_response("a1", &day("10:00"), "effort-test", 1),
            model_response("a2", &day("10:05"), "effort-test", 1),
            model_response("a3", &day("10:10"), "other-model", 100),
        ],
    );
    // Equal responses: more output tokens win.
    seed(
        &mut db,
        "b",
        &[
            model_response("b1", &day("11:00"), "effort-test", 5),
            model_response("b2", &day("11:04"), "other-model", 10),
        ],
    );
    // No response at all: no model recorded.
    seed(
        &mut db,
        "c",
        &[event("c1", &day("12:00")), event("c2", &day("12:03"))],
    );
    // Equal responses and output: the first name.
    seed(
        &mut db,
        "d",
        &[
            model_response("d1", &day("13:00"), "other-model", 3),
            model_response("d2", &day("13:01"), "effort-test", 3),
        ],
    );
    let report = report(&db, false);
    let day = report
        .cohort
        .by_day
        .iter()
        .find(|d| d.date == "2026-09-02")
        .unwrap();
    assert_eq!(
        day.models,
        [
            ModelDayEffort {
                model: None,
                priced_nano_usd: 0,
                priced_observations: 0,
                unpriced_observations: 0,
                agent_ms: 180_000,
            },
            ModelDayEffort {
                model: Some("effort-test".into()),
                priced_nano_usd: 10_000_000_000,
                priced_observations: 4,
                unpriced_observations: 0,
                agent_ms: 660_000,
            },
            ModelDayEffort {
                model: Some("other-model".into()),
                priced_nano_usd: 0,
                priced_observations: 0,
                unpriced_observations: 3,
                agent_ms: 240_000,
            },
        ]
    );
    assert!(
        report
            .cohort
            .by_day
            .iter()
            .filter(|d| d.date != "2026-09-02")
            .all(|d| d.models.is_empty())
    );
}

/// A session's hours follow the model it used most over the whole range,
/// even on a day it used another model more; that day's cost still goes to
/// the model of each response.
#[test]
fn pr_effort_hours_follow_the_range_model_not_the_day_model() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            // Day one: effort-test twice, other-model never.
            model_response("s1", "2026-09-02T10:00:00Z", "effort-test", 1),
            model_response("s2", "2026-09-02T10:05:00Z", "effort-test", 1),
            // Day two: other-model three times, so it is the range's most used.
            model_response("s3", "2026-09-03T10:00:00Z", "other-model", 1),
            model_response("s4", "2026-09-03T10:05:00Z", "other-model", 1),
            model_response("s5", "2026-09-03T10:10:00Z", "other-model", 1),
        ],
    );
    let report = report(&db, false);
    let day = |date: &str| {
        &report
            .cohort
            .by_day
            .iter()
            .find(|d| d.date == date)
            .unwrap()
            .models
    };
    assert_eq!(
        day("2026-09-02"),
        &[
            ModelDayEffort {
                model: Some("effort-test".into()),
                priced_nano_usd: 2_000_000_000,
                priced_observations: 2,
                unpriced_observations: 0,
                agent_ms: 0,
            },
            ModelDayEffort {
                model: Some("other-model".into()),
                priced_nano_usd: 0,
                priced_observations: 0,
                unpriced_observations: 0,
                agent_ms: 300_000,
            },
        ]
    );
    assert_eq!(
        day("2026-09-03"),
        &[ModelDayEffort {
            model: Some("other-model".into()),
            priced_nano_usd: 0,
            priced_observations: 0,
            unpriced_observations: 3,
            agent_ms: 600_000,
        }]
    );
}
