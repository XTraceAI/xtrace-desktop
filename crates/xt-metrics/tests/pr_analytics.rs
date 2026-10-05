use jiff::Timestamp;
use rusqlite::Connection;
use serde_json::json;
use xt_fixtures::TempDb;
use xt_metrics::{
    MetricsDb, PrAnalyticsReport, PrFreshness, PrRow, SessionWindow, TokenWithheld, Window,
};
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource,
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
fn now() -> i64 {
    ms("2026-09-08T00:00:00Z")
}
fn seed(db: &mut TempDb, id: &str, surface: &str, rows: &[CanonicalRecord]) {
    seed_on(db, id, "claude", surface, rows);
}
fn seed_on(db: &mut TempDb, id: &str, host: &str, surface: &str, rows: &[CanonicalRecord]) {
    let mut session = SessionMeta::new(id, host, SessionSource::Fixture);
    session.surface = Some(surface.into());
    db.store_mut().upsert_session(&session, false).unwrap();
    db.store_mut().upsert_records(id, rows, false).unwrap();
}
fn human(id: &str, ts: &str) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":id,"type":"user","timestamp":ts,
        "message":{"role":"user","content":[{"type":"text","text":"Synthetic"}]}}))
    .unwrap()
}
fn plain(id: &str, ts: &str) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":id,"type":"assistant","timestamp":ts,
        "message":{"role":"assistant","content":[{"type":"text","text":"Synthetic"}]}}))
    .unwrap()
}
/// An assistant tool call, optionally carrying one response's usage:
/// input 1, the given output, zero cache counters, and a model.
fn tool(id: &str, ts: &str, output: Option<Option<u64>>, model: Option<&str>) -> CanonicalRecord {
    let mut message = json!({"role":"assistant","content":[{"type":"tool_use","id":format!("tool-{id}"),"name":"Read","input":{"file_path":"synthetic"}}]});
    if let Some(output) = output {
        let mut usage =
            json!({"input_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0});
        if let Some(output) = output {
            usage["output_tokens"] = json!(output);
        }
        message["usage"] = usage;
    }
    if let Some(model) = model {
        message["model"] = json!(model);
    }
    serde_json::from_value(json!({"uuid":id,"type":"assistant","timestamp":ts,"message":message}))
        .unwrap()
}
const MODEL: Option<&str> = Some("claude-synthetic");
/// A human message followed `minutes` later by a measured tool response.
fn stretch(prefix: &str, at: &str, minutes: i64, output: u64) -> Vec<CanonicalRecord> {
    let start = at.parse::<Timestamp>().unwrap();
    let end = start
        .checked_add(jiff::SignedDuration::from_mins(minutes))
        .unwrap();
    vec![
        human(&format!("{prefix}-h"), &start.to_string()),
        tool(
            &format!("{prefix}-t"),
            &end.to_string(),
            Some(Some(output)),
            MODEL,
        ),
    ]
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
    state: PrState,
    at: Option<&str>,
    title: &str,
    head: &str,
) {
    db.store_mut()
        .record_pr_refresh(&RefreshOutcome::Success(RefreshSuccess {
            pull_request: PrIdentity::from_url(&url(number)).unwrap(),
            attempted_at: 100,
            title: title.into(),
            state,
            merged_at: at.map(Into::into),
            additions: 1,
            deletions: 1,
            head_ref_name: head.into(),
        }))
        .unwrap();
}
fn merged(db: &mut TempDb, number: u64, at: &str, title: &str) {
    refresh(db, number, PrState::Merged, Some(at), title, "main");
}
fn fail(db: &mut TempDb, number: u64, error: PrRefreshError) {
    db.store_mut()
        .record_pr_refresh(&RefreshOutcome::Failure(RefreshFailure {
            pull_request: PrIdentity::from_url(&url(number)).unwrap(),
            attempted_at: 200,
            error,
        }))
        .unwrap();
}
fn raw(db: &TempDb, sql: &str) {
    let c = Connection::open(db.path()).unwrap();
    xt_store::timestamp::register_sqlite(&c).unwrap();
    c.execute_batch(sql).unwrap();
}
fn report_in(db: &TempDb, window: Window, confirmed_only: bool) -> PrAnalyticsReport {
    MetricsDb::open(db.path())
        .unwrap()
        .pr_analytics(window, now(), confirmed_only)
        .unwrap()
}
fn report(db: &TempDb, confirmed_only: bool) -> PrAnalyticsReport {
    report_in(db, window(), confirmed_only)
}
fn row(report: &PrAnalyticsReport, number: u64) -> &PrRow {
    report
        .rows
        .iter()
        .find(|row| row.number == number)
        .unwrap_or_else(|| panic!("no PR {number} in {:?}", report.rows))
}
fn numbers(report: &PrAnalyticsReport) -> Vec<u64> {
    report.rows.iter().map(|row| row.number).collect()
}

/// The canonical per-session value a row must contain, unchanged, once.
fn session(db: &TempDb, id: &str) -> SessionWindow {
    MetricsDb::open(db.path())
        .unwrap()
        .session_windows(window(), &[id])
        .unwrap()
        .remove(id)
        .unwrap()
}

#[test]
fn shared_sessions_overlap_and_confidence_filters_before_membership() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "shared",
        "cli",
        &stretch("shared", "2026-09-07T12:00:00Z", 1, 100),
    );
    seed(
        &mut db,
        "sha",
        "cli",
        &stretch("sha", "2026-09-07T13:00:00Z", 2, 10),
    );
    seed(
        &mut db,
        "inf",
        "cli",
        &stretch("inf", "2026-09-07T14:00:00Z", 4, 1000),
    );
    link(&mut db, "shared", 1, PrConfidence::Exact);
    // A second, weaker witness of the same link neither duplicates the
    // session nor weakens its evidence.
    link(&mut db, "shared", 1, PrConfidence::Inferred);
    link(&mut db, "sha", 1, PrConfidence::Sha);
    link(&mut db, "inf", 1, PrConfidence::Inferred);
    link(&mut db, "shared", 2, PrConfidence::Exact);
    link(&mut db, "inf", 3, PrConfidence::Inferred);
    merged(&mut db, 1, "2026-09-05T00:00:00Z", "feat: one");
    merged(&mut db, 2, "2026-09-06T00:00:00Z", "fix: two");
    merged(&mut db, 3, "2026-09-04T00:00:00Z", "docs: three");

    let all = report(&db, false);
    assert_eq!(numbers(&all), [3, 1, 2], "merge order");
    let one = row(&all, 1);
    assert_eq!((one.linked_sessions, one.active_sessions), (3, 3));
    assert_eq!(
        (one.evidence.exact, one.evidence.sha, one.evidence.inferred),
        (1, 1, 1)
    );
    assert_eq!(one.confidence, PrConfidence::Exact);
    assert_eq!(one.tokens.counters.total_tokens, Some(101 + 11 + 1001));
    assert_eq!(one.tokens.counters.output_tokens, Some(100 + 10 + 1000));
    assert_eq!(one.human_messages, Some(3));
    assert_eq!(one.agent_ms, Some(7 * 60_000));
    assert_eq!(
        (one.hands_off.n, one.hands_off.median_min),
        (Some(3), Some(2.0))
    );

    // The shared session contributes its whole canonical value to both rows.
    let SessionWindow::Indexed {
        tokens,
        agent_ms,
        human_messages,
        ..
    } = session(&db, "shared")
    else {
        panic!("shared is indexed");
    };
    let two = row(&all, 2);
    assert_eq!(two.tokens.counters, tokens.counters);
    assert_eq!(
        (two.agent_ms, two.human_messages),
        (Some(agent_ms), human_messages)
    );
    assert_eq!(two.linked_sessions, 1);
    assert_eq!(row(&all, 3).evidence.inferred, 1);

    // Per-PR medians have PR sample sizes; there is no total to add rows into.
    let tokens = &all.summary.tokens;
    assert_eq!(tokens.withheld, None);
    assert_eq!(
        (tokens.median.eligible_prs, tokens.median.measured_prs),
        (3, 3)
    );
    assert_eq!(tokens.median.median, Some(1001.0));
    assert_eq!(all.summary.hands_off_min.median, Some(2.0));
    let value = serde_json::to_value(&all).unwrap();
    assert!(value.get("total").is_none() && value["summary"].get("total").is_none());

    // confirmed_only removes each inferred link before membership: the mixed
    // row loses only its inferred session, the inferred-only PR disappears.
    let confirmed = report(&db, true);
    assert_eq!(numbers(&confirmed), [1, 2]);
    assert_eq!(confirmed.eligibility.merged, 2);
    assert_eq!(
        confirmed.eligibility.outside + confirmed.eligibility.unknown_facts,
        0
    );
    let one = row(&confirmed, 1);
    assert_eq!(one.linked_sessions, 2);
    assert_eq!(
        (one.evidence.exact, one.evidence.sha, one.evidence.inferred),
        (1, 1, 0)
    );
    assert_eq!(one.tokens.counters.total_tokens, Some(101 + 11));
    assert_eq!(
        (one.hands_off.n, one.hands_off.median_min),
        (Some(2), Some(1.5))
    );
    assert_eq!(row(&confirmed, 2), two);
    let types: Vec<_> = confirmed
        .by_type
        .iter()
        .map(|group| (group.work_type.as_deref(), group.summary.prs))
        .collect();
    assert_eq!(types, [(Some("feat"), 1), (Some("fix"), 1)]);
}

#[test]
fn half_open_boundaries_cached_facts_and_title_first_types() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "edge",
        "cli",
        &[
            human("before", "2026-08-31T23:59:59Z"),
            tool("start", "2026-09-01T00:00:00Z", Some(Some(5)), MODEL),
            tool("end", "2026-09-08T00:00:00Z", Some(Some(7)), MODEL),
        ],
    );
    for number in 10..=17 {
        link(&mut db, "edge", number, PrConfidence::Exact);
    }
    refresh(
        &mut db,
        10,
        PrState::Merged,
        Some("2026-09-01T00:00:00Z"),
        "Update",
        "fix/x",
    );
    merged(&mut db, 11, "2026-09-08T00:00:00Z", "feat: at the end");
    refresh(&mut db, 12, PrState::Open, None, "feat: open", "feat/o");
    // 13 was never refreshed.
    merged(&mut db, 14, "2026-09-02T00:00:00Z", "feat: no instant");
    refresh(
        &mut db,
        15,
        PrState::Merged,
        Some("2026-09-03T00:00:00Z"),
        "fix: y",
        "feat/z",
    );
    fail(&mut db, 15, PrRefreshError::RateLimited);
    fail(&mut db, 16, PrRefreshError::RateLimited);
    merged(&mut db, 17, "2026-09-02T00:00:00Z", "feat: untitled");
    raw(
        &db,
        &format!(
            "UPDATE pull_requests SET title=NULL WHERE url='{}';
             UPDATE pull_requests SET merged_at=NULL WHERE url='{}'",
            url(17),
            url(14)
        ),
    );

    let report = report(&db, false);
    assert_eq!(numbers(&report), [10, 17, 15]);
    let facts = &report.eligibility;
    assert_eq!(
        (facts.merged, facts.outside, facts.unknown_facts),
        (3, 2, 3)
    );
    assert_eq!(facts.unresolved_type, 1);
    assert_eq!(
        (
            facts.freshness.never_attempted,
            facts.freshness.failed_never_refreshed,
            facts.freshness.failed_after_refresh
        ),
        (1, 1, 1)
    );
    assert_eq!(
        row(&report, 10).work_type.as_deref(),
        Some("fix"),
        "branch fallback"
    );
    assert_eq!(
        row(&report, 15).work_type.as_deref(),
        Some("fix"),
        "title first"
    );
    assert!(matches!(
        row(&report, 15).freshness,
        PrFreshness::FailedAfterRefresh(_)
    ));
    assert_eq!(row(&report, 17).work_type, None);
    let types: Vec<_> = report
        .by_type
        .iter()
        .map(|group| (group.work_type.as_deref(), group.summary.prs))
        .collect();
    assert_eq!(types, [(None, 1), (Some("fix"), 2)]);

    // Only the event at the start instant is in the window.
    let edge = row(&report, 10);
    assert_eq!((edge.linked_sessions, edge.active_sessions), (1, 1));
    assert_eq!(edge.tokens.counters.total_tokens, Some(6));
    assert_eq!(edge.human_messages, Some(0), "measured zero");
    assert_eq!(edge.agent_ms, Some(0));
    assert_eq!(
        (edge.hands_off.n, edge.hands_off.median_min),
        (Some(0), None)
    );
    let hands_off = &report.summary.hands_off_min;
    assert_eq!((hands_off.no_sample_prs, hands_off.unknown_prs), (3, 0));
    assert_eq!((hands_off.measured_median, hands_off.median), (None, None));

    // Zero eligible pull requests is an empty, truthful report.
    let empty = report_in(
        &db,
        Window::new(ms("2026-07-01T00:00:00Z"), ms("2026-07-08T00:00:00Z")).unwrap(),
        false,
    );
    assert!(empty.rows.is_empty() && empty.by_type.is_empty());
    assert_eq!(empty.eligibility.unknown_facts, 3);
    assert_eq!(empty.summary.tokens.median.eligible_prs, 0);
    assert_eq!(empty.summary.tokens.median.median, None);
}

#[test]
fn unknowns_known_zeros_and_inactive_members_stay_distinct() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "zero",
        "cli",
        &[
            human("zero-h", "2026-09-07T12:00:00Z"),
            tool("zero-t", "2026-09-07T12:01:00Z", Some(Some(0)), MODEL),
        ],
    );
    raw(&db, "UPDATE usage SET input_tokens=0 WHERE uuid='zero-t'");
    seed(
        &mut db,
        "idle",
        "cli",
        &stretch("idle", "2026-08-10T12:00:00Z", 1, 1),
    );
    seed(
        &mut db,
        "partial",
        "cli",
        &[
            human("partial-h", "2026-09-07T13:00:00Z"),
            tool("partial-t", "2026-09-07T13:01:00Z", Some(None), MODEL),
        ],
    );
    seed(
        &mut db,
        "unclassified",
        "cli",
        &stretch("unc", "2026-09-07T14:00:00Z", 3, 8),
    );
    raw(&db, "UPDATE records SET is_human=NULL WHERE uuid='unc-h'");
    seed(
        &mut db,
        "judge",
        "cli",
        &stretch("judge", "2026-09-07T15:00:00Z", 1, 1),
    );
    raw(
        &db,
        "UPDATE sessions SET kind='judge' WHERE session_id='judge'",
    );
    for (session, number) in [
        ("zero", 20),
        ("zero", 21),
        ("idle", 21),
        ("partial", 22),
        ("judge", 22),
        ("unclassified", 23),
    ] {
        link(&mut db, session, number, PrConfidence::Exact);
    }
    for number in 20..=23 {
        merged(&mut db, number, "2026-09-05T00:00:00Z", "feat: x");
    }
    let report = report(&db, false);

    let zero = row(&report, 20);
    assert_eq!(zero.tokens.counters.total_tokens, Some(0), "known zero");
    assert_eq!(zero.human_messages, Some(1));

    // An inactive linked member keeps its no-selected-usage unknown.
    let idle = row(&report, 21);
    assert_eq!((idle.linked_sessions, idle.active_sessions), (2, 1));
    assert_eq!(idle.tokens.counters.total_tokens, None);
    assert_eq!(idle.tokens.counters.output_tokens, None);
    assert_eq!(idle.tokens.no_selected_usage_sessions, 1);
    assert_eq!(idle.tokens.measured_sessions, 1);
    assert_eq!(
        (idle.human_messages, idle.agent_ms),
        (Some(1), Some(60_000))
    );

    // Incomplete counters poison the total, not the measured components; a
    // judge identity is disclosed and never measured.
    let partial = row(&report, 22);
    assert_eq!(partial.tokens.counters.total_tokens, None);
    assert_eq!(partial.tokens.counters.input_tokens, Some(1));
    assert_eq!(partial.tokens.incomplete_sessions, 1);
    assert_eq!((partial.linked_sessions, partial.unmeasured_links), (1, 1));

    let unclassified = row(&report, 23);
    assert_eq!(unclassified.human_messages, None);
    assert_eq!(unclassified.human_unknown_sessions, 1);
    assert_eq!(unclassified.tokens.counters.total_tokens, Some(1 + 8));
    assert_eq!(unclassified.hands_off.n, None);
    assert_eq!(unclassified.hands_off.unknown_sessions, 1);

    // Unknown rows keep measured-only medians labeled apart from the cohort.
    let human = &report.summary.human_messages;
    assert_eq!(
        (human.eligible_prs, human.measured_prs, human.unknown_prs),
        (4, 3, 1)
    );
    assert_eq!((human.measured_median, human.median), (Some(1.0), None));
    let agent = &report.summary.agent_ms;
    assert_eq!((agent.measured_prs, agent.unknown_prs), (4, 0));
    assert_eq!(agent.median, Some(60_000.0));

    // One incomplete of three eligible sessions fails the gate: the token
    // median is withheld while its counts stay, and nothing else is gated.
    let gate = &report.token_gate;
    assert_eq!((gate.eligible_sessions, gate.measured_sessions), (3, 2));
    assert_eq!(gate.passes, Some(false));
    let tokens = &report.summary.tokens;
    assert_eq!(tokens.withheld, Some(TokenWithheld::GateFailed));
    assert_eq!(
        (tokens.median.measured_prs, tokens.median.unknown_prs),
        (2, 2)
    );
    assert_eq!(
        (tokens.median.measured_median, tokens.median.median),
        (None, None)
    );
}

#[test]
fn token_gate_is_fixed_fourteen_days_exactly_ninety_percent_passes() {
    let build = |unknown_model: usize| {
        let mut db = TempDb::empty().unwrap();
        for index in 0..10 {
            let model = if index < unknown_model { None } else { MODEL };
            let id = format!("s{index}");
            seed(
                &mut db,
                &id,
                "cli",
                &[
                    human(&format!("{id}-h"), "2026-09-07T12:00:00Z"),
                    tool(
                        &format!("{id}-t"),
                        "2026-09-07T12:01:00Z",
                        Some(Some(4)),
                        model,
                    ),
                ],
            );
        }
        // Structurally unmeasured: named, never in the denominator.
        seed_on(
            &mut db,
            "cursor",
            "cursor",
            "cursor-cli",
            &[plain("c", "2026-09-07T12:00:00Z")],
        );
        link(&mut db, "s0", 1, PrConfidence::Exact);
        merged(&mut db, 1, "2026-09-05T00:00:00Z", "feat: x");
        db
    };
    let passing = build(1);
    let report = report(&passing, false);
    assert_eq!(report.token_gate.pct, Some(90.0));
    assert_eq!(report.token_gate.passes, Some(true));
    assert_eq!(report.token_gate.excluded_surfaces.len(), 1);
    // Unknown model still yields a measured token total.
    assert_eq!(row(&report, 1).tokens.counters.total_tokens, Some(5));
    assert_eq!(report.summary.tokens.median.median, Some(5.0));
    // A longer display window does not move the fixed gate.
    let wider = report_in(
        &passing,
        Window::new(ms("2026-08-09T00:00:00Z"), now()).unwrap(),
        false,
    );
    assert_eq!(wider.token_gate, report.token_gate);

    let failing = report_in(&build(2), window(), false);
    assert_eq!(failing.token_gate.passes, Some(false));
    assert_eq!(
        failing.summary.tokens.withheld,
        Some(TokenWithheld::GateFailed)
    );
    assert_eq!(row(&failing, 1).tokens.counters.total_tokens, Some(5));

    // No eligible denominator is unknown, not a pass.
    let mut empty = TempDb::empty().unwrap();
    seed(
        &mut empty,
        "old",
        "cli",
        &stretch("old", "2026-08-01T12:00:00Z", 1, 1),
    );
    link(&mut empty, "old", 1, PrConfidence::Exact);
    merged(&mut empty, 1, "2026-09-05T00:00:00Z", "feat: x");
    let unknown = report_in(&empty, window(), false);
    assert_eq!(unknown.token_gate.passes, None);
    assert_eq!(
        unknown.summary.tokens.withheld,
        Some(TokenWithheld::GateUnknown)
    );
}

#[test]
fn hands_off_pools_stretches_instead_of_taking_a_median_of_medians() {
    let mut db = TempDb::empty().unwrap();
    let mut x = stretch("x1", "2026-09-07T10:00:00Z", 1, 1);
    x.extend(stretch("x2", "2026-09-07T10:10:00Z", 2, 1));
    x.extend(stretch("x3", "2026-09-07T10:20:00Z", 3, 1));
    seed(&mut db, "x", "cli", &x);
    seed(
        &mut db,
        "y",
        "cli",
        &stretch("y", "2026-09-07T11:00:00Z", 10, 1),
    );
    link(&mut db, "x", 30, PrConfidence::Exact);
    link(&mut db, "y", 30, PrConfidence::Exact);
    link(&mut db, "y", 31, PrConfidence::Exact);
    merged(&mut db, 30, "2026-09-05T00:00:00Z", "feat: a");
    merged(&mut db, 31, "2026-09-05T00:00:01Z", "feat: b");
    let report = report(&db, false);
    // Session medians are 2 and 10, whose median would be 6.
    let pooled = row(&report, 30);
    assert_eq!(
        (pooled.hands_off.n, pooled.hands_off.median_min),
        (Some(4), Some(2.5))
    );
    assert_eq!(pooled.hands_off.measured_sessions, 2);
    assert_eq!(row(&report, 31).hands_off.median_min, Some(10.0));
    let summary = &report.summary.hands_off_min;
    assert_eq!((summary.measured_prs, summary.median), (2, Some(6.25)));
}

#[test]
fn surface_health_is_judged_with_unlinked_siblings_and_unrelated_unknowns_do_not_poison() {
    let mut db = TempDb::empty().unwrap();
    // Linked, healthy on its own: three stretches on distinct instants.
    let mut linked = stretch("l1", "2026-09-07T10:00:00Z", 1, 1);
    linked.extend(stretch("l2", "2026-09-07T10:10:00Z", 2, 1));
    linked.extend(stretch("l3", "2026-09-07T10:20:00Z", 3, 1));
    seed(&mut db, "linked", "desktop", &linked);
    // Two unlinked siblings whose six records share one instant.
    for sibling in ["d1", "d2"] {
        let rows: Vec<_> = (0..6)
            .map(|i| plain(&format!("{sibling}-{i}"), "2026-09-07T09:00:00Z"))
            .collect();
        seed(&mut db, sibling, "desktop", &rows);
    }
    seed(
        &mut db,
        "healthy",
        "cli",
        &stretch("h", "2026-09-07T12:00:00Z", 5, 1),
    );
    // An unrelated session with an unknown classification on the same surface.
    seed(
        &mut db,
        "unrelated",
        "cli",
        &stretch("u", "2026-09-07T13:00:00Z", 1, 1),
    );
    raw(&db, "UPDATE records SET is_human=NULL WHERE uuid='u-h'");
    // Linked but without a tool call: known to have no stretch.
    seed(
        &mut db,
        "quiet",
        "cli",
        &[
            human("q-h", "2026-09-07T14:00:00Z"),
            plain("q-a", "2026-09-07T14:05:00Z"),
        ],
    );
    link(&mut db, "linked", 40, PrConfidence::Exact);
    link(&mut db, "linked", 41, PrConfidence::Exact);
    link(&mut db, "healthy", 41, PrConfidence::Exact);
    link(&mut db, "quiet", 42, PrConfidence::Exact);
    for number in 40..=42 {
        merged(&mut db, number, "2026-09-05T00:00:00Z", "feat: x");
    }
    let report = report(&db, false);
    let excluded = &report.hands_off_excluded_surfaces;
    assert_eq!(excluded.len(), 1);
    assert_eq!(excluded[0].surface.as_deref(), Some("desktop"));
    assert_eq!(
        (
            excluded[0].qualifying_sessions,
            excluded[0].degenerate_sessions
        ),
        (3, 2)
    );

    let only_excluded = row(&report, 40);
    assert_eq!(only_excluded.hands_off.excluded_sessions, 1);
    assert_eq!(only_excluded.hands_off.excluded_surfaces, *excluded);
    assert_eq!(only_excluded.hands_off.median_min, None);
    let mixed = row(&report, 41);
    assert_eq!(
        (mixed.hands_off.n, mixed.hands_off.median_min),
        (Some(1), Some(5.0))
    );
    assert_eq!(mixed.hands_off.excluded_surfaces, *excluded);
    let quiet = row(&report, 42);
    assert_eq!(
        (quiet.hands_off.n, quiet.hands_off.median_min),
        (Some(0), None)
    );

    // Excluded-only is not a known absence; no stretches is.
    let summary = &report.summary.hands_off_min;
    assert_eq!(
        (
            summary.measured_prs,
            summary.unknown_prs,
            summary.no_sample_prs
        ),
        (1, 1, 1)
    );
    assert_eq!((summary.measured_median, summary.median), (Some(5.0), None));

    // Judged among the linked session alone, the surface would be healthy.
    let lone = TempDb::empty().unwrap();
    let mut lone = lone;
    seed(&mut lone, "linked", "desktop", &linked);
    link(&mut lone, "linked", 40, PrConfidence::Exact);
    merged(&mut lone, 40, "2026-09-05T00:00:00Z", "feat: x");
    let alone = report_in(&lone, window(), false);
    assert_eq!(row(&alone, 40).hands_off.n, Some(3));
}

#[test]
fn many_overlapping_prs_over_more_than_two_hundred_sessions_read_in_bounded_batches() {
    const SESSIONS: usize = 450;
    let mut db = TempDb::empty().unwrap();
    let ids: Vec<String> = (0..SESSIONS).map(|i| format!("s{i:03}")).collect();
    for (index, id) in ids.iter().enumerate() {
        let at = Timestamp::from_millisecond(ms("2026-09-02T00:00:00Z") + index as i64 * 600_000)
            .unwrap()
            .to_string();
        seed(&mut db, id, "cli", &stretch(id, &at, 1, 2));
        link(&mut db, id, 999, PrConfidence::Exact);
    }
    for pr in 0..40_usize {
        for offset in 0..60 {
            let id = &ids[(pr * 10 + offset) % SESSIONS];
            link(&mut db, id, pr as u64 + 100, PrConfidence::Sha);
        }
        merged(&mut db, pr as u64 + 100, "2026-09-05T00:00:00Z", "fix: x");
    }
    merged(&mut db, 999, "2026-09-06T00:00:00Z", "feat: all");

    let metrics = MetricsDb::open(db.path()).unwrap();
    let started = std::time::Instant::now();
    let report = metrics.pr_analytics(window(), now(), false).unwrap();
    let elapsed = started.elapsed();
    eprintln!("{SESSIONS} sessions / 41 PRs: {elapsed:?}");
    assert!(elapsed < std::time::Duration::from_secs(5), "{elapsed:?}");

    assert_eq!(report.rows.len(), 41);
    let all = row(&report, 999);
    assert_eq!(all.linked_sessions, SESSIONS as u64);
    assert_eq!(all.tokens.counters.total_tokens, Some(3 * SESSIONS as u64));
    assert_eq!(all.human_messages, Some(SESSIONS as u64));
    assert_eq!(all.agent_ms, Some(60_000 * SESSIONS as u64));
    assert_eq!(
        (all.hands_off.n, all.hands_off.median_min),
        (Some(SESSIONS as u64), Some(1.0))
    );
    for pr in 100..140 {
        let row = row(&report, pr);
        assert_eq!(row.linked_sessions, 60);
        assert_eq!(row.evidence.sha, 60);
        assert_eq!(row.tokens.counters.total_tokens, Some(180));
    }
    // The canonical measurements agree when read in explicit bounded batches.
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut total = 0;
    for batch in refs.chunks(xt_metrics::MAX_SESSIONS) {
        for value in metrics.session_windows(window(), batch).unwrap().values() {
            let SessionWindow::Indexed { tokens, .. } = value else {
                panic!("indexed");
            };
            total += tokens.counters.total_tokens.unwrap();
        }
    }
    assert_eq!(all.tokens.counters.total_tokens, Some(total));
    assert!(metrics.session_windows(window(), &refs).is_err());
}
