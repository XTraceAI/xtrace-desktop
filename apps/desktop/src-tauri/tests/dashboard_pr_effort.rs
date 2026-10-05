//! The Dashboard's M-19 section is the core report, converted field for field,
//! read in the same snapshot as the rest of the Dashboard for the selected and
//! the previous equal window, with inferred links removed first.
#![cfg(all(debug_assertions, feature = "fixtures"))]
use jiff::{Timestamp, tz::TimeZone};
use serde_json::json;
use xt_fixtures::TempDb;
use xt_metrics::{MetricsDb, PriceCatalog, TypingRate, Window};
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource,
    pr_link::{
        PrConfidence, PrIdentity, PrLinkObservation, PrRefreshError, PrState, RefreshFailure,
        RefreshOutcome, RefreshSuccess,
    },
};
use xtrace_desktop::{dashboard::assemble, dto::*};

const DAY: i64 = 86_400_000;

fn ms(text: &str) -> i64 {
    text.parse::<Timestamp>().unwrap().as_millisecond()
}
fn now() -> i64 {
    ms("2026-09-08T00:00:00Z")
}
fn report(db: &TempDb) -> DashboardMetrics {
    assemble(
        &MetricsDb::open(db.path()).unwrap(),
        7,
        now(),
        TimeZone::UTC,
        MetricClock::Fixture,
        &PriceCatalog::bundled().unwrap(),
        TypingRate::default(),
    )
    .unwrap()
}
/// The core report the section must equal, for the same window and filter.
fn core(db: &TempDb, window: Window) -> serde_json::Value {
    let report = MetricsDb::open(db.path())
        .unwrap()
        .pr_effort(
            window,
            TimeZone::UTC,
            true,
            &PriceCatalog::bundled().unwrap(),
        )
        .unwrap();
    serde_json::to_value(report).unwrap()
}
fn current_window() -> Window {
    Window::new(now() - 7 * DAY, now()).unwrap()
}
/// The bundled Claude Opus 5.5 standard output rate, in nano-USD per token.
const OUTPUT_NANO_USD: u64 = 20_000;
/// The dollars `output` tokens cost when nothing else is charged, converted
/// once from nano-USD as the core report converts.
fn usd(output: u64) -> f64 {
    (output * OUTPUT_NANO_USD) as f64 / 1_000_000_000.0
}
/// One priced response with no input or cache tokens; a missing output
/// counter leaves it unpriced.
fn response(uuid: &str, timestamp: &str, output: Option<u64>) -> CanonicalRecord {
    let mut usage = json!({"input_tokens":0,"cache_read_input_tokens":0,
        "cache_creation_input_tokens":0,"service_tier":"standard"});
    if let Some(output) = output {
        usage["output_tokens"] = json!(output);
    }
    serde_json::from_value(json!({"uuid":uuid,"type":"assistant","timestamp":timestamp,
        "message":{"role":"assistant","model":"claude-opus-5-5","content":[],"usage":usage}}))
    .unwrap()
}
fn session(db: &mut TempDb, id: &str, rows: &[CanonicalRecord]) {
    let session = SessionMeta::new(id, "claude", SessionSource::Fixture);
    db.store_mut().upsert_session(&session, false).unwrap();
    db.store_mut().upsert_records(id, rows, false).unwrap();
}
fn identity(number: u64) -> PrIdentity {
    PrIdentity::from_url(&format!("https://github.com/xtrace/app/pull/{number}")).unwrap()
}
fn link(db: &mut TempDb, session: &str, number: u64, confidence: PrConfidence) {
    db.store_mut()
        .record_pr_link(&PrLinkObservation {
            session_id: session.into(),
            pull_request: identity(number),
            confidence,
            first_seen_at: 1,
            last_seen_at: 2,
        })
        .unwrap();
}
fn merged(db: &mut TempDb, number: u64, merged_at: &str, title: &str) {
    db.store_mut()
        .record_pr_refresh(&RefreshOutcome::Success(RefreshSuccess {
            pull_request: identity(number),
            attempted_at: 100,
            title: title.into(),
            state: PrState::Merged,
            merged_at: Some(merged_at.into()),
            additions: 1,
            deletions: 1,
            head_ref_name: "main".into(),
        }))
        .unwrap();
}
fn fail(db: &mut TempDb, number: u64, at: i64, error: PrRefreshError) {
    db.store_mut()
        .record_pr_refresh(&RefreshOutcome::Failure(RefreshFailure {
            pull_request: identity(number),
            attempted_at: at,
            error,
        }))
        .unwrap();
}
fn assignments(section: &DashboardPrEffort) -> Vec<(MetricEffortAssignment, u64, Option<f64>)> {
    section
        .current
        .by_assignment
        .iter()
        .map(|group| {
            (
                group.assignment.clone(),
                group.effort.sessions,
                group.effort.cost.total_usd,
            )
        })
        .collect()
}
fn ty(name: &str) -> MetricEffortAssignment {
    MetricEffortAssignment::Type(name.into())
}
/// Every M-19 number the Dashboard carries is the core report's.
fn assert_core_parity(db: &TempDb, report: &DashboardMetrics) {
    let section = &report.pr_effort;
    assert_eq!(section.rule_id, "M-19");
    assert!(section.current.confirmed_only);
    assert_eq!(
        serde_json::to_value(&section.current).unwrap(),
        core(db, current_window())
    );
    assert_eq!(
        serde_json::to_value(&section.previous).unwrap(),
        core(db, current_window().previous().unwrap())["tile"]
    );
    assert_eq!(
        report.tiles.merged_prs.value,
        section.current.tile.merged.map(|n| n as f64)
    );
    assert_eq!(report.tiles.merged_prs.rule_id, "M-19");
    assert!(!report.unavailable.iter().any(|u| u.key == "merged_prs"));
    assert!(!report.unavailable.iter().any(|u| u.key == "work_type"));
}

/// F19: one 100k-output session linked exactly to two merged pull requests of
/// different types. Both count in the tile, each once on its merge day; the
/// session's output counts once, under `mixed`, on its own event day.
#[test]
fn f19_shared_session_counts_once_as_mixed_on_its_event_day() {
    let mut db = TempDb::empty().unwrap();
    session(
        &mut db,
        "f19",
        &[response("r1", "2026-09-03T10:00:00Z", Some(100_000))],
    );
    link(&mut db, "f19", 1, PrConfidence::Exact);
    link(&mut db, "f19", 2, PrConfidence::Exact);
    merged(&mut db, 1, "2026-09-05T12:00:00Z", "feat: add the thing");
    merged(&mut db, 2, "2026-09-06T12:00:00Z", "fix: repair the thing");
    let report = report(&db);
    assert_core_parity(&db, &report);
    let section = &report.pr_effort;
    assert_eq!(
        (section.current.tile.merged, section.current.tile.complete),
        (Some(2), true)
    );
    assert_eq!(report.tiles.merged_prs.value, Some(2.0));
    assert_eq!(
        assignments(section),
        [(MetricEffortAssignment::Mixed, 1, Some(usd(100_000)))]
    );
    let dates: Vec<_> = section
        .current
        .markers
        .iter()
        .map(|m| (m.number, m.date.as_str(), m.work_type.as_deref()))
        .collect();
    assert_eq!(
        dates,
        [
            (1, "2026-09-05", Some("feat")),
            (2, "2026-09-06", Some("fix"))
        ]
    );
    let mixed = &section.current.by_assignment[0].effort;
    let output_days: Vec<_> = mixed
        .by_day
        .iter()
        .filter(|day| day.cost.priced_subtotal_usd > 0.0)
        .map(|day| day.date.as_str())
        .collect();
    assert_eq!(output_days, ["2026-09-03"]);
    assert_eq!(section.current.cohort.cost.total_usd, Some(usd(100_000)));
}

/// An inferred link is removed before eligibility and assignment: the pull
/// request it names is neither counted nor typed.
#[test]
fn inferred_links_are_removed_before_assignment() {
    let mut db = TempDb::empty().unwrap();
    session(
        &mut db,
        "f19",
        &[response("r1", "2026-09-03T10:00:00Z", Some(100_000))],
    );
    link(&mut db, "f19", 1, PrConfidence::Exact);
    link(&mut db, "f19", 2, PrConfidence::Inferred);
    merged(&mut db, 1, "2026-09-05T12:00:00Z", "feat: add the thing");
    merged(&mut db, 2, "2026-09-06T12:00:00Z", "fix: repair the thing");
    let report = report(&db);
    assert_core_parity(&db, &report);
    let section = &report.pr_effort;
    assert_eq!(section.current.tile.merged, Some(1));
    assert_eq!(assignments(section), [(ty("feat"), 1, Some(usd(100_000)))]);
    assert_eq!(section.current.markers.len(), 1);
    assert_eq!(
        section.current.markers[0].confidence,
        MetricPrConfidence::Exact
    );
}

/// A linked pull request with no cached facts leaves the count unknown: the
/// known merged subtotal is named, never shown as the answer or as zero, and
/// the session it could type is unresolved.
#[test]
fn unknown_facts_keep_the_known_subtotal_and_leave_the_count_unknown() {
    let mut db = TempDb::empty().unwrap();
    session(
        &mut db,
        "known",
        &[response("r1", "2026-09-03T10:00:00Z", Some(10))],
    );
    session(
        &mut db,
        "unknown",
        &[response("r2", "2026-09-04T10:00:00Z", Some(20))],
    );
    link(&mut db, "known", 1, PrConfidence::Exact);
    link(&mut db, "unknown", 3, PrConfidence::Sha);
    merged(&mut db, 1, "2026-09-05T12:00:00Z", "chore: tidy");
    let report = report(&db);
    assert_core_parity(&db, &report);
    let tile = &report.pr_effort.current.tile;
    assert_eq!(
        (
            tile.known_merged,
            tile.unknown_facts,
            tile.complete,
            tile.merged
        ),
        (1, 1, false, None)
    );
    assert_eq!(tile.freshness.never_attempted, 1);
    let merged_prs = &report.tiles.merged_prs;
    assert_eq!(merged_prs.value, None);
    assert_eq!(
        merged_prs.reason.as_deref(),
        Some("1 known merged; 1 linked pull request has no cached merge facts")
    );
    assert!(merged_prs.delta.suppressed);
    assert_eq!(
        assignments(&report.pr_effort),
        [
            (ty("chore"), 1, Some(usd(10))),
            (MetricEffortAssignment::Unresolved, 1, Some(usd(20)))
        ]
    );
}

/// No linked pull request at all is a measured zero, and unlinked work is
/// `other`; an unmeasured output stays unknown rather than becoming zero.
#[test]
fn no_links_is_a_measured_zero_and_unmeasured_output_stays_unknown() {
    let mut db = TempDb::empty().unwrap();
    session(
        &mut db,
        "plain",
        &[response("r1", "2026-09-03T10:00:00Z", None)],
    );
    let report = report(&db);
    assert_core_parity(&db, &report);
    assert_eq!(report.tiles.merged_prs.value, Some(0.0));
    assert_eq!(report.tiles.merged_prs.reason, None);
    let section = &report.pr_effort;
    assert_eq!(
        assignments(section),
        [(MetricEffortAssignment::Other, 1, None)]
    );
    let cost = &section.current.cohort.cost;
    assert_eq!(
        (
            cost.total_usd,
            cost.selected_observations,
            cost.priced_observations,
            cost.priced_subtotal_usd
        ),
        (None, 1, 0, 0.0)
    );
    assert_eq!(
        cost.unpriced[0].reason,
        MetricUnpricedReason::MissingCounters
    );
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(
        json["pr_effort"]["current"]["cohort"]["cost"]["total_usd"],
        serde_json::Value::Null
    );
}

/// The previous equal window is read in the same snapshot; the delta compares
/// two complete counts with at least five pull requests on each side.
#[test]
fn the_previous_window_supplies_the_delta() {
    let mut db = TempDb::empty().unwrap();
    session(
        &mut db,
        "work",
        &[response("r1", "2026-09-03T10:00:00Z", Some(1))],
    );
    for number in 1..=10 {
        link(&mut db, "work", number, PrConfidence::Exact);
        merged(&mut db, number, "2026-09-05T12:00:00Z", "feat: current");
    }
    for number in 11..=15 {
        link(&mut db, "work", number, PrConfidence::Exact);
        merged(&mut db, number, "2026-08-28T12:00:00Z", "feat: previous");
    }
    let report = report(&db);
    assert_core_parity(&db, &report);
    assert_eq!(report.pr_effort.previous.merged, Some(5));
    let tile = &report.tiles.merged_prs;
    assert_eq!((tile.value, tile.delta.previous), (Some(10.0), Some(5.0)));
    assert_eq!((tile.current_n, tile.previous_n), (Some(10), Some(5)));
    assert_eq!(
        (tile.delta.pct, tile.delta.suppressed),
        (Some(100.0), false)
    );
}

/// A refresh that fails after a success keeps that success's facts and says
/// so, beside the tile, the marker and the freshness summary.
#[test]
fn cached_success_survives_a_later_failure() {
    let mut db = TempDb::empty().unwrap();
    session(
        &mut db,
        "work",
        &[response("r1", "2026-09-03T10:00:00Z", Some(5))],
    );
    link(&mut db, "work", 1, PrConfidence::Exact);
    merged(&mut db, 1, "2026-09-05T12:00:00Z", "feat: kept");
    fail(&mut db, 1, 200, PrRefreshError::RateLimited);
    let report = report(&db);
    assert_core_parity(&db, &report);
    let section = &report.pr_effort.current;
    assert_eq!(section.tile.merged, Some(1));
    assert_eq!(section.tile.freshness.failed_after_refresh, 1);
    assert_eq!(
        section.markers[0].freshness,
        MetricPrFreshness::FailedAfterRefresh(PrRefreshErrorCode::RateLimited)
    );
    assert_eq!(
        assignments(&report.pr_effort),
        [(ty("feat"), 1, Some(usd(5)))]
    );
}
