use jiff::Timestamp;
use serde_json::json;
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, Store};
use xtrace_desktop::state::{AppState, StartupOptions};

fn seed(store: &mut Store, id: &str, rows: &[CanonicalRecord]) {
    let mut session = SessionMeta::new(id, "claude", SessionSource::Fixture);
    session.surface = Some("cli".into());
    store.upsert_session(&session, false).unwrap();
    store.upsert_records(id, rows, false).unwrap();
}
/// One Claude response snapshot, addressable by its (message id, request id).
fn response(
    uuid: &str,
    timestamp: &str,
    request: &str,
    usage: serde_json::Value,
) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":uuid,"type":"assistant","timestamp":timestamp,
        "requestId":request,
        "message":{"id":format!("response-{request}"),"role":"assistant",
            "model":"test-claude","content":[],"usage":usage}}))
    .unwrap()
}
fn usage(output: u64) -> serde_json::Value {
    json!({"input_tokens":1000,"output_tokens":output,"cache_read_input_tokens":0,
        "cache_creation_input_tokens":0,"service_tier":"standard"})
}

#[cfg(all(debug_assertions, feature = "fixtures"))]
mod summary {
    use super::*;
    use jiff::tz::TimeZone;
    use std::path::PathBuf;
    use xt_fixtures::{Fixture, TempDb};
    use xt_metrics::{MetricsDb, PriceCatalog};
    use xtrace_desktop::{
        dashboard::fixture_catalog,
        dto::MetricClock,
        state::StateError,
        today::{TodayCostState, TodayOutputState, TodaySummary, today},
    };

    fn event(uuid: &str, timestamp: &str) -> CanonicalRecord {
        serde_json::from_value(json!({"uuid":uuid,"type":"assistant","timestamp":timestamp,
            "message":{"role":"assistant","model":"test-claude","content":[]}}))
        .unwrap()
    }
    fn catalog() -> PriceCatalog {
        let fixture =
            Fixture::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/F1"))
                .unwrap();
        fixture_catalog(fixture.snapshots().get("prices")).unwrap()
    }
    fn ms(text: &str) -> i64 {
        text.parse::<Timestamp>().unwrap().as_millisecond()
    }
    fn read(db: &TempDb, now: i64, zone: TimeZone) -> TodaySummary {
        today(
            &MetricsDb::open(db.path()).unwrap(),
            now,
            zone,
            MetricClock::System,
            &catalog(),
        )
        .unwrap()
    }

    #[test]
    fn today_reads_local_midnight_to_now_and_nothing_after() {
        let mut db = TempDb::empty().unwrap();
        seed(
            db.store_mut(),
            "day",
            &[
                // Yesterday, local: excluded.
                response("y", "2026-09-07T23:59:59.999Z", "r0", usage(1000)),
                // Midnight is included; the span runs to 00:10.
                response("a", "2026-09-08T00:00:00Z", "r1", usage(7)),
                response("b", "2026-09-08T00:10:00Z", "r2", usage(5)),
                // Exactly now and later are outside [midnight, now).
                response("now", "2026-09-08T12:00:00Z", "r3", usage(100)),
                response("future", "2026-09-08T13:00:00Z", "r4", usage(100)),
            ],
        );
        let summary = read(&db, ms("2026-09-08T12:00:00Z"), TimeZone::UTC);
        assert_eq!(summary.date, "2026-09-08");
        assert_eq!(
            (
                summary.start_ms,
                summary.observed_ms,
                summary.next_midnight_ms
            ),
            (
                ms("2026-09-08T00:00:00Z"),
                ms("2026-09-08T12:00:00Z"),
                ms("2026-09-09T00:00:00Z")
            )
        );
        assert!(!summary.empty);
        assert_eq!(summary.timezone, "UTC");
        assert_eq!(summary.output.state, TodayOutputState::Recorded);
        assert_eq!(summary.output.output_tokens, Some(12));
        assert_eq!(
            (summary.output.selected_responses, summary.output.sessions),
            (2, 1)
        );
        assert_eq!(summary.cost.state, TodayCostState::Priced);
        assert!(summary.cost.total_usd.unwrap() > 0.0);
        assert_eq!(
            summary.cost.total_usd,
            Some(summary.cost.priced_subtotal_usd)
        );
        assert_eq!(summary.cost.price_version, "synthetic-v1");
        assert_eq!(
            (summary.agent.active_ms, summary.agent.sessions),
            (600_000, 1)
        );
        let unavailable: Vec<&str> = summary.unavailable.iter().map(|u| u.key.as_str()).collect();
        assert_eq!(unavailable, ["active_now", "last_rule_fire"]);
    }

    #[test]
    fn today_is_the_local_date_across_a_dst_transition() {
        let mut db = TempDb::empty().unwrap();
        seed(
            db.store_mut(),
            "dst",
            &[
                // 23:30 EDT on Oct 31: the previous local date.
                response("before", "2026-11-01T03:30:00Z", "r1", usage(1000)),
                // 00:30 EDT and 01:30 EST: both on the 25-hour date.
                response("early", "2026-11-01T04:30:00Z", "r2", usage(3)),
                response("repeat", "2026-11-01T06:30:00Z", "r3", usage(4)),
            ],
        );
        let zone = TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();
        let summary = read(&db, ms("2026-11-01T20:00:00Z"), zone);
        assert_eq!(summary.date, "2026-11-01");
        assert_eq!(summary.start_ms, ms("2026-11-01T04:00:00Z"));
        assert_eq!(summary.next_midnight_ms - summary.start_ms, 25 * 3_600_000);
        assert_eq!(summary.output.output_tokens, Some(7));
        // A POSIX rule names no IANA zone: the report says so.
        assert_eq!(summary.timezone, "system-local");
    }

    #[test]
    fn today_at_exact_midnight_is_explicitly_empty() {
        let mut db = TempDb::empty().unwrap();
        seed(
            db.store_mut(),
            "edge",
            &[
                response("late", "2026-09-07T23:50:00Z", "r1", usage(9)),
                response("midnight", "2026-09-08T00:00:00Z", "r2", usage(9)),
            ],
        );
        let now = ms("2026-09-08T00:00:00Z");
        let summary = read(&db, now, TimeZone::UTC);
        assert!(summary.empty);
        assert_eq!(summary.date, "2026-09-08");
        assert_eq!((summary.start_ms, summary.observed_ms), (now, now));
        assert_eq!(summary.next_midnight_ms, ms("2026-09-09T00:00:00Z"));
        assert_eq!(summary.output.state, TodayOutputState::NoneRecorded);
        assert_eq!(summary.output.output_tokens, None);
        assert_eq!(summary.cost.state, TodayCostState::NoneRecorded);
        assert_eq!(summary.cost.total_usd, None);
        assert_eq!(summary.cost.priced_subtotal_usd, 0.0);
        assert_eq!(summary.cost.price_version, "synthetic-v1");
        assert_eq!((summary.agent.active_ms, summary.agent.sessions), (0, 0));
        // One millisecond later the midnight response is today's.
        let after = read(&db, now + 1, TimeZone::UTC);
        assert!(!after.empty);
        assert_eq!(after.output.output_tokens, Some(9));
    }

    #[test]
    fn today_counts_a_copied_response_once_at_its_latest_snapshot() {
        let mut db = TempDb::empty().unwrap();
        seed(
            db.store_mut(),
            "copies",
            &[
                // One response snapshotted yesterday, then revised today:
                // selected once, today, at the revised counters.
                response("rev-a", "2026-09-07T23:00:00Z", "shared", usage(5)),
                response("rev-b", "2026-09-08T01:00:00Z", "shared", usage(9)),
                // Two snapshots of another, both today.
                response("dup-a", "2026-09-08T02:00:00Z", "twice", usage(1)),
                response("dup-b", "2026-09-08T02:00:01Z", "twice", usage(2)),
            ],
        );
        // The same response copied into a second session's history.
        seed(
            db.store_mut(),
            "copy",
            &[response(
                "copy-a",
                "2026-09-08T02:00:02Z",
                "twice",
                usage(3),
            )],
        );
        let summary = read(&db, ms("2026-09-08T12:00:00Z"), TimeZone::UTC);
        assert_eq!(summary.output.selected_responses, 2);
        assert_eq!(summary.output.output_tokens, Some(12));
        assert_eq!(summary.cost.selected_observations, 2);
        // Activity is per session, so both sessions were active today.
        assert_eq!(summary.agent.sessions, 2);
    }

    #[test]
    fn today_states_partial_unknown_and_none_recorded_separately() {
        let mut db = TempDb::empty().unwrap();
        seed(
            db.store_mut(),
            "mixed",
            &[
                response("priced", "2026-09-08T01:00:00Z", "r1", usage(10)),
                // Priced output, but no service tier: unpriced.
                response(
                    "untiered",
                    "2026-09-08T01:05:00Z",
                    "r2",
                    json!({"input_tokens":1,"output_tokens":4,
                        "cache_read_input_tokens":0,"cache_creation_input_tokens":0}),
                ),
            ],
        );
        let now = ms("2026-09-08T12:00:00Z");
        let partial = read(&db, now, TimeZone::UTC);
        assert_eq!(partial.output.state, TodayOutputState::Recorded);
        assert_eq!(partial.output.output_tokens, Some(14));
        assert_eq!(partial.cost.state, TodayCostState::Partial);
        assert_eq!(partial.cost.total_usd, None);
        assert!(partial.cost.priced_subtotal_usd > 0.0);
        assert_eq!(
            (
                partial.cost.priced_observations,
                partial.cost.selected_observations
            ),
            (1, 2)
        );
        // A response without an output counter makes today's output unknown.
        seed(
            db.store_mut(),
            "no-output",
            &[response(
                "input-only",
                "2026-09-08T02:00:00Z",
                "r3",
                json!({"input_tokens":1}),
            )],
        );
        let incomplete = read(&db, now, TimeZone::UTC);
        assert_eq!(incomplete.output.state, TodayOutputState::Incomplete);
        assert_eq!(incomplete.output.output_tokens, None);
        assert_eq!(incomplete.output.selected_responses, 3);

        // Activity without any usage: agent time measured, output none recorded.
        let mut quiet = TempDb::empty().unwrap();
        seed(
            quiet.store_mut(),
            "quiet",
            &[
                event("q1", "2026-09-08T03:00:00Z"),
                event("q2", "2026-09-08T03:15:00Z"),
            ],
        );
        let none = read(&quiet, now, TimeZone::UTC);
        assert_eq!(none.output.state, TodayOutputState::NoneRecorded);
        assert_eq!(none.cost.state, TodayCostState::NoneRecorded);
        assert_eq!((none.agent.active_ms, none.agent.sessions), (900_000, 1));
        // Responses selected, none of them priceable.
        let mut unpriced = TempDb::empty().unwrap();
        seed(
            unpriced.store_mut(),
            "unpriced",
            &[response(
                "u",
                "2026-09-08T03:00:00Z",
                "u1",
                json!({"output_tokens":2}),
            )],
        );
        let unpriced = read(&unpriced, now, TimeZone::UTC);
        assert_eq!(unpriced.cost.state, TodayCostState::Unpriced);
        assert_eq!(unpriced.output.output_tokens, Some(2));
    }

    #[test]
    fn today_agent_time_is_the_dashboards_bar_for_today() {
        // 23:50 and 00:10 in one session: a span over midnight. The tray and
        // the Dashboard's bar for today both count its 10 minutes after
        // midnight; a span ending before midnight counts for neither.
        let mut db = TempDb::empty().unwrap();
        seed(
            db.store_mut(),
            "night",
            &[
                event("n1", "2026-09-07T23:50:00Z"),
                event("n2", "2026-09-08T00:10:00Z"),
            ],
        );
        seed(
            db.store_mut(),
            "evening",
            &[
                event("e1", "2026-09-07T22:00:00Z"),
                event("e2", "2026-09-07T22:15:00Z"),
            ],
        );
        let now = ms("2026-09-08T12:00:00Z");
        let summary = read(&db, now, TimeZone::UTC);
        assert_eq!(
            (summary.agent.active_ms, summary.agent.sessions),
            (600_000, 1)
        );
        for days in xtrace_desktop::dashboard::WINDOW_PRESETS {
            let report = xtrace_desktop::dashboard::assemble(
                &MetricsDb::open(db.path()).unwrap(),
                days,
                now,
                TimeZone::UTC,
                MetricClock::System,
                &catalog(),
                xt_metrics::TypingRate::default(),
                xt_metrics::BreakLength::default(),
            )
            .unwrap();
            let bar = report.days.last().unwrap();
            assert_eq!(bar.date, summary.date);
            assert_eq!((bar.start_ms, bar.end_ms), (summary.start_ms, now));
            assert_eq!(
                (bar.agent_hours * 3_600_000.0).round() as u64,
                summary.agent.active_ms,
                "{days}"
            );
        }
    }

    /// Tokens, cost and spans are read in one snapshot, by construction.
    #[test]
    fn today_reads_stay_inside_one_snapshot_by_construction() {
        let module = include_str!("../src/today.rs");
        let (_, body) = module.split_once("pub fn today(").expect("today");
        let (_, closure) = body
            .split_once("db.read_snapshot(|db| {")
            .expect("one snapshot");
        let (closure, _) = closure.split_once("})?;").expect("snapshot end");
        for read in [
            "db.tokens(window",
            "db.cost(window",
            "db.active_last_day(window",
        ] {
            assert_eq!(module.matches(read).count(), 1, "{read} is read elsewhere");
            assert!(closure.contains(read), "{read} is outside the snapshot");
        }
        assert_eq!(module.matches("read_snapshot(").count(), 1);
    }

    #[test]
    fn today_native_fixture_is_the_pinned_empty_midnight() {
        let root = tempfile::TempDir::new().unwrap();
        let state = AppState::build(
            StartupOptions {
                data_dir: Some(root.path().into()),
                fixture: Some("F1".into()),
                ..Default::default()
            },
            || panic!("fixture must not resolve live directory"),
            || panic!("fixture must not resolve the home directory"),
        )
        .unwrap();
        let summary = state.today().unwrap();
        // F1 is pinned at 2026-09-08T00:00:00Z, so the fixture's today holds
        // no instant yet, and its seven days of history stay out of it.
        assert!(summary.empty);
        assert_eq!(summary.clock, MetricClock::Fixture);
        assert_eq!(
            (summary.date.as_str(), summary.timezone.as_str()),
            ("2026-09-08", "UTC")
        );
        assert_eq!(summary.observed_ms, ms("2026-09-08T00:00:00Z"));
        assert_eq!(summary.output.state, TodayOutputState::NoneRecorded);
        assert_eq!(summary.cost.price_version, "synthetic-v1");
        state.shutdown();
        assert!(matches!(state.today(), Err(StateError::Closed)));
    }
}

#[test]
fn today_native_state_uses_the_system_clock_and_rereads_commits() {
    let root = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(root.path().join("data")),
            native_home: Some(root.path().join("home")),
            ..Default::default()
        },
        || panic!("explicit data directory"),
        || panic!("explicit native home"),
    )
    .unwrap();
    let before = state.today().unwrap();
    assert_eq!(before.clock, xtrace_desktop::dto::MetricClock::System);
    assert!(before.start_ms <= before.observed_ms && before.observed_ms < before.next_midnight_ms);
    let now = Timestamp::now();
    assert!((now.as_millisecond() - before.observed_ms).abs() < 60_000);
    assert_eq!(
        before.output.state,
        xtrace_desktop::today::TodayOutputState::NoneRecorded
    );
    // A committed write between two reads is seen by the second. Skip the
    // insert when the local day is under a second old: "just now" would be
    // yesterday.
    if before.observed_ms - before.start_ms > 2_000 {
        let at = Timestamp::from_millisecond(before.observed_ms - 1_000)
            .unwrap()
            .to_string();
        let mut writer = Store::open(root.path().join("data/xtrace.db")).unwrap();
        seed(
            &mut writer,
            "live",
            &[response("live", &at, "l1", usage(42))],
        );
        let after = state.today().unwrap();
        assert_eq!(after.output.output_tokens, Some(42));
        assert_eq!(after.cost.price_version, before.cost.price_version);
    }
}
