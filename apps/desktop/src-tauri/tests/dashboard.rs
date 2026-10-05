use jiff::Timestamp;
use serde_json::json;
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, Store};
use xtrace_desktop::state::{AppState, StartupOptions, StateError};

fn assistant(uuid: &str, timestamp: &str, usage: Option<serde_json::Value>) -> CanonicalRecord {
    let mut value = json!({"uuid":uuid,"type":"assistant","timestamp":timestamp,
        "message":{"role":"assistant","model":"test-claude","content":[]}});
    if let Some(usage) = usage {
        value["message"]["usage"] = usage;
    }
    serde_json::from_value(value).unwrap()
}
fn seed(store: &mut Store, id: &str, host: &str, surface: Option<&str>, rows: &[CanonicalRecord]) {
    let mut session = SessionMeta::new(id, host, SessionSource::Fixture);
    session.surface = surface.map(str::to_owned);
    store.upsert_session(&session, false).unwrap();
    store.upsert_records(id, rows, false).unwrap();
}

#[cfg(all(debug_assertions, feature = "fixtures"))]
mod assembled {
    use super::*;
    use jiff::tz::TimeZone;
    use std::path::PathBuf;
    use xt_fixtures::{Fixture, TempDb};
    use xt_metrics::{MetricsDb, PriceCatalog, TypingRate};
    use xt_store::pr_link::{
        PrConfidence, PrIdentity, PrLinkObservation, PrState, RefreshOutcome, RefreshSuccess,
    };
    use xtrace_desktop::{
        dashboard::{LANE_LIMIT, LANE_WINDOW_MS, assemble, fixture_catalog},
        dto::*,
    };

    fn fixture(id: &str) -> Fixture {
        Fixture::load(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../fixtures")
                .join(id),
        )
        .unwrap()
    }
    fn catalog() -> PriceCatalog {
        fixture_catalog(fixture("F1").snapshots().get("prices")).unwrap()
    }
    fn ms(text: &str) -> i64 {
        text.parse::<Timestamp>().unwrap().as_millisecond()
    }
    fn now() -> i64 {
        ms("2026-09-08T00:00:00Z")
    }
    fn report(db: &TempDb, days: u32) -> DashboardMetrics {
        let metrics = MetricsDb::open(db.path()).unwrap();
        assemble(
            &metrics,
            days,
            now(),
            TimeZone::UTC,
            MetricClock::Fixture,
            &catalog(),
            TypingRate::default(),
        )
        .unwrap()
    }
    fn records(value: &serde_json::Value) -> Vec<CanonicalRecord> {
        serde_json::from_value(value.clone()).unwrap()
    }

    #[test]
    fn dashboard_f2_concurrency_lanes_and_agent_time_for_each_preset() {
        let f = fixture("F2");
        let mut db = TempDb::empty().unwrap();
        for lane in f.snapshots()["spans"]["lanes"].as_array().unwrap() {
            let mut session = f.sessions()[0].metadata.clone();
            session.session_id = lane["session_id"].as_str().unwrap().into();
            db.store_mut().upsert_session(&session, false).unwrap();
            db.store_mut()
                .upsert_records(&session.session_id, &records(&lane["records"]), false)
                .unwrap();
        }
        for days in [7, 14, 30] {
            let report = report(&db, days);
            assert_eq!(
                (
                    report.window.days,
                    report.window.start_ms,
                    report.window.end_ms
                ),
                (days, now() - i64::from(days) * 86_400_000, now())
            );
            assert_eq!(report.window.timezone, "UTC");
            assert_eq!(report.window.clock, MetricClock::Fixture);
            assert_eq!(report.days.len(), days as usize);
            assert_eq!(report.tiles.concurrency_max.value, Some(3.0));
            assert_eq!(report.tiles.concurrency_mean.value, Some(1.8));
            assert_eq!(report.tiles.concurrency_max.rule_id, "M-06");
            // Ninety additive agent minutes over fifty wall minutes.
            assert_eq!(report.tiles.agent_hours.value, Some(1.5));
            assert_eq!(
                report.tiles.agent_hours_per_day.value,
                Some(1.5 / f64::from(days))
            );
            assert!(
                report
                    .tiles
                    .agent_hours_per_day
                    .note
                    .as_deref()
                    .unwrap()
                    .contains(&format!("divided by {days} reported local date buckets"))
            );
            assert_eq!(
                (report.lane_start_ms, report.lane_end_ms),
                (now() - LANE_WINDOW_MS, now())
            );
            assert_eq!((report.lanes_total, report.lanes_truncated), (3, false));
            let ids: Vec<_> = report.lanes.iter().map(|l| l.session_id.as_str()).collect();
            assert_eq!(ids, ["f2-lane-a", "f2-lane-c", "f2-lane-b"]);
            assert!(report.lanes.iter().all(|lane| {
                report
                    .lanes
                    .iter()
                    .filter(|other| other.start_ms < lane.end_ms && lane.start_ms < other.end_ms)
                    .count()
                    > 1
            }));
            // Sample counts are below five: the delta is suppressed, not invented.
            assert_eq!(report.tiles.sessions.current_n, Some(3));
            assert!(report.tiles.sessions.delta.suppressed);
            assert_eq!(report.tiles.sessions.delta.previous, Some(0.0));
        }
    }

    #[test]
    fn dashboard_lane_cap_limits_display_rows_only() {
        let mut db = TempDb::empty().unwrap();
        for index in 0..(LANE_LIMIT + 5) {
            let id = format!("lane-{index:03}");
            seed(
                db.store_mut(),
                &id,
                "claude",
                Some("cli"),
                &[
                    assistant(&format!("{id}-a"), "2026-09-07T20:00:00Z", None),
                    assistant(&format!("{id}-b"), "2026-09-07T20:10:00Z", None),
                ],
            );
        }
        // Inside the selected range but outside the fixed 48-hour lane axis.
        seed(
            db.store_mut(),
            "older",
            "claude",
            Some("cli"),
            &[
                assistant("older-a", "2026-09-03T20:00:00Z", None),
                assistant("older-b", "2026-09-03T20:10:00Z", None),
            ],
        );
        let report = report(&db, 7);
        assert_eq!(report.lanes.len(), LANE_LIMIT);
        assert_eq!(report.lanes_total, LANE_LIMIT as u64 + 5);
        assert!(report.lanes_truncated);
        assert_eq!(report.lanes[0].session_id, "lane-000");
        assert_eq!(report.lanes[LANE_LIMIT - 1].session_id, "lane-199");
        assert!(report.lanes.iter().all(|lane| lane.session_id != "older"));
        assert_eq!(
            report.tiles.concurrency_max.value,
            Some((LANE_LIMIT + 5) as f64)
        );
        assert_eq!(report.tiles.sessions.value, Some((LANE_LIMIT + 6) as f64));
    }

    #[test]
    fn dashboard_f7_unmeasured_usage_stays_null_with_named_reasons() {
        let f = fixture("F7");
        let data = &f.snapshots()["coverage"];
        let mut db = TempDb::empty().unwrap();
        seed(
            db.store_mut(),
            &f.sessions()[0].metadata.session_id,
            "cursor",
            data["surface"].as_str(),
            &records(&data["records"]),
        );
        let report = report(&db, 7);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["tiles"]["tokens"]["value"], serde_json::Value::Null);
        assert!(report.tiles.tokens.reason.is_some());
        assert_eq!(json["cost"]["total_usd"], serde_json::Value::Null);
        assert_eq!(report.tiles.sessions.value, Some(1.0));
        assert_eq!(
            (
                report.usage_coverage.total.sessions,
                report.usage_coverage.total.measured,
                report.usage_coverage.total.pct
            ),
            (1, data["expected_measured"].as_u64().unwrap(), Some(0.0))
        );
        assert!(
            report
                .usage_coverage
                .total
                .gaps
                .contains(&MetricUsageGap::NoSelectedUsage)
        );
        assert_eq!(
            report.usage_coverage.by_surface[0].surface.as_deref(),
            Some("cursor-cli")
        );
        // Receipts and index state never make the runtime inventory known.
        assert_eq!(report.capture_inventory, MetricInventory::Unknown);
        let rule_fires = &report.tiles.rule_fires;
        assert_eq!(rule_fires.value, None);
        assert!(rule_fires.reason.is_some());
        assert_eq!(rule_fires.sample_unit, "unavailable");
        assert!(rule_fires.delta.suppressed);
        // No session links a pull request: every retained fact is known, so
        // zero merged is a measured zero, not an unavailable metric.
        let merged = &report.tiles.merged_prs;
        assert_eq!(
            (
                merged.value,
                merged.reason.as_deref(),
                merged.rule_id.as_str()
            ),
            (Some(0.0), None, "M-19")
        );
        assert!(merged.delta.suppressed);
        let keys: Vec<_> = report.unavailable.iter().map(|u| u.key.as_str()).collect();
        assert_eq!(keys, ["rule_fires", "environment"]);
        assert_eq!(report.favorite.rule_id, "M-10");
        assert_eq!(report.favorite.current.model, None);
        assert!(report.favorite.current.unknown_reason.is_some());
    }

    #[test]
    fn dashboard_empty_database_distinguishes_measured_zero_from_unknown() {
        let db = TempDb::empty().unwrap();
        let report = report(&db, 30);
        let tiles = &report.tiles;
        for zero in [
            &tiles.sessions,
            &tiles.agent_hours,
            &tiles.agent_hours_per_day,
            &tiles.sessions_per_day,
        ] {
            assert_eq!((zero.value, zero.reason.as_deref()), (Some(0.0), None));
        }
        for unknown in [
            &tiles.ratio,
            &tiles.concurrency_max,
            &tiles.concurrency_mean,
            &tiles.hands_off_median,
            &tiles.hands_off_p90,
            &tiles.cost,
        ] {
            assert_eq!(unknown.value, None);
            assert!(unknown.reason.is_some());
        }
        assert_eq!(report.usage_coverage.total.pct, None);
        assert_eq!(report.usage_gate_14d.pct, None);
        assert_eq!((report.lanes.len(), report.lanes_total), (0, 0));
        assert!(report.capture_coverage.is_empty());
        // Nothing indexed at all, so nothing indexed is missing a timestamp:
        // a measured zero with no surface rows, never an unknown.
        assert_eq!(
            report.untimed_history,
            DashboardUntimed {
                records: 0,
                by_surface: vec![]
            }
        );
        assert!(report.tokens_by_host.is_empty());
        assert_eq!(report.days.len(), 30);
        assert!(report.days.iter().all(|day| day.sessions == 0));
        // Nothing was recorded on any day, which is a measurement: the hero's
        // own zeros, split thirty ways, never an unknown.
        assert!(
            report
                .days
                .iter()
                .all(|day| (day.agent_hours, day.human_hours_est) == (0.0, Some(0.0)))
        );
    }

    /// One untimed record per uuid, whatever the source did or did not state.
    fn untimed(uuid: &str) -> CanonicalRecord {
        serde_json::from_value(json!({"uuid":uuid,"type":"assistant",
            "message":{"role":"assistant","model":"test-claude","content":[]}}))
        .unwrap()
    }

    #[test]
    fn dashboard_untimed_history_is_all_indexed_history_and_ignores_the_range() {
        let mut db = TempDb::empty().unwrap();
        seed(
            db.store_mut(),
            "claude-cli",
            "claude",
            Some("cli"),
            &[
                assistant("timed", "2026-09-07T12:00:00Z", None),
                untimed("11111111-1111-4111-8111-111111111111"),
                untimed("22222222-2222-4222-8222-222222222222"),
            ],
        );
        // A second host whose surface the source never stated.
        seed(
            db.store_mut(),
            "cursor-unknown",
            "cursor",
            None,
            &[untimed("33333333-3333-4333-8333-333333333333")],
        );
        let expected = DashboardUntimed {
            records: 3,
            by_surface: vec![
                MetricUntimedSurface {
                    host: "claude".into(),
                    surface: Some("cli".into()),
                    records: 2,
                },
                MetricUntimedSurface {
                    host: "cursor".into(),
                    surface: None,
                    records: 1,
                },
            ],
        };
        for days in [7, 14, 30] {
            let report = report(&db, days);
            // The same disclosure whatever range is selected: these records are
            // in no day, so no range can include or exclude them.
            assert_eq!(report.untimed_history, expected, "{days}d");
            assert_eq!(
                report.untimed_history.records,
                report
                    .untimed_history
                    .by_surface
                    .iter()
                    .map(|row| row.records)
                    .sum::<u64>(),
                "{days}d"
            );
            // And the windowed measurements still hold only the timed record:
            // the disclosure explains that gap rather than closing it.
            assert_eq!(report.tiles.sessions.value, Some(1.0), "{days}d");
            assert_eq!(
                report.days.iter().map(|day| day.sessions).sum::<u64>(),
                1,
                "{days}d"
            );
        }
    }

    #[test]
    fn dashboard_f11_low_sample_deltas_keep_values_and_previous_favorite() {
        let f = fixture("F11");
        let mut db = TempDb::empty().unwrap();
        for session in f.snapshots()["favorite"]["sessions"].as_array().unwrap() {
            seed(
                db.store_mut(),
                session["session_id"].as_str().unwrap(),
                "claude",
                Some("cli"),
                &records(&session["records"]),
            );
        }
        let low = report(&db, 7);
        assert_eq!(low.favorite.current.model.as_deref(), Some("B"));
        assert_eq!(low.favorite.previous.model.as_deref(), Some("A"));
        let sessions = &low.tiles.sessions;
        assert_eq!(
            (sessions.value, sessions.current_n, sessions.previous_n),
            (Some(1.0), Some(1), Some(1))
        );
        assert_eq!(
            (
                sessions.delta.previous,
                sessions.delta.pct,
                sessions.delta.suppressed
            ),
            (Some(1.0), None, true)
        );
        assert_eq!(low.tiles.human_messages.value, Some(2.0));
        assert_eq!(low.tiles.human_messages.delta.previous, Some(3.0));
        assert!(low.tiles.human_messages.delta.suppressed);

        // Ten current and five previous sessions: +100 percentage points.
        for (prefix, day, count) in [("current", "2026-09-05", 9), ("previous", "2026-08-28", 4)] {
            for index in 0..count {
                let id = format!("{prefix}-{index}");
                seed(
                    db.store_mut(),
                    &id,
                    "claude",
                    Some("cli"),
                    &[assistant(
                        &format!("{id}-a"),
                        &format!("{day}T12:00:00Z"),
                        None,
                    )],
                );
            }
        }
        let shown = report(&db, 7);
        let sessions = &shown.tiles.sessions;
        assert_eq!(
            (sessions.current_n, sessions.previous_n),
            (Some(10), Some(5))
        );
        assert_eq!(
            (
                sessions.delta.previous,
                sessions.delta.pct,
                sessions.delta.suppressed
            ),
            (Some(5.0), Some(100.0), false)
        );
    }

    #[test]
    fn dashboard_partial_pricing_and_usage_coverage_keep_subtotal_separate() {
        let counters = |tier: Option<&str>| {
            json!({"input_tokens":1000,"output_tokens":100,"cache_read_input_tokens":0,
                "cache_creation_input_tokens":0,"service_tier":tier})
        };
        let mut db = TempDb::empty().unwrap();
        for (id, usage) in [
            ("priced", counters(Some("standard"))),
            ("untiered", counters(None)),
            ("partial", json!({"output_tokens":5})),
        ] {
            seed(
                db.store_mut(),
                id,
                "claude",
                Some("cli"),
                &[assistant(id, "2026-09-07T12:00:00Z", Some(usage))],
            );
        }
        let report = report(&db, 7);
        let cost = &report.cost;
        assert_eq!(cost.total_usd, None);
        assert_eq!(report.tiles.cost.value, None);
        assert_eq!(report.tiles.cost.unit, "usd_api_equivalent");
        assert!(cost.priced_subtotal_usd > 0.0);
        assert_eq!(cost.price_version, "synthetic-v1");
        assert!(!cost.basis.is_empty() && !cost.as_of.is_empty());
        assert_eq!(cost.selected_observations, 3);
        assert_eq!(cost.priced_observations, 1);
        assert_eq!(cost.unpriced_observations, 2);
        assert!(
            cost.unpriced
                .iter()
                .any(|u| u.reason == MetricUnpricedReason::MissingServiceTier
                    && u.model.as_deref() == Some("test-claude")
                    && u.service_tier.is_none())
        );
        let usage = &report.usage_coverage.total;
        assert_eq!((usage.sessions, usage.measured), (3, 2));
        assert!((usage.pct.unwrap() - 200.0 / 3.0).abs() < 1e-9);
        assert!(usage.gaps.contains(&MetricUsageGap::IncompleteCounters));
        // The day series carries the same partial subtotal and unknown total.
        let day = report.days.iter().find(|d| d.date == "2026-09-07").unwrap();
        assert_eq!(
            (day.cost_usd, day.priced_subtotal_usd, day.sessions),
            (None, cost.priced_subtotal_usd, 3)
        );
        assert_eq!(report.tokens_by_host.len(), 1);
        assert_eq!(report.tokens_by_host[0].host, "claude");
    }

    #[test]
    fn dashboard_cost_delta_samples_usage_sessions_not_responses() {
        let usage = |input: u64| {
            json!({"input_tokens":input,"output_tokens":10,"cache_read_input_tokens":0,
                "cache_creation_input_tokens":0,"service_tier":"standard"})
        };
        let mut db = TempDb::empty().unwrap();
        // One session per window, each with at least five priced responses.
        for (id, day, responses, input) in [
            ("current", "2026-09-06", 6, 2000),
            ("previous", "2026-08-30", 5, 1000),
        ] {
            let rows: Vec<_> = (0..responses)
                .map(|index| {
                    assistant(
                        &format!("{id}-{index}"),
                        &format!("{day}T12:0{index}:00Z"),
                        Some(usage(input)),
                    )
                })
                .collect();
            seed(db.store_mut(), id, "claude", Some("cli"), &rows);
        }
        let report = report(&db, 7);
        let cost = &report.tiles.cost;
        assert_eq!(report.cost.selected_observations, 6);
        assert!(cost.value.is_some() && cost.delta.previous.is_some());
        assert_ne!(cost.value, cost.delta.previous);
        assert_eq!(
            (cost.current_n, cost.previous_n, cost.sample_unit.as_str()),
            (Some(1), Some(1), "sessions")
        );
        assert_eq!((cost.delta.pct, cost.delta.suppressed), (None, true));
        assert_eq!(
            (
                report.tiles.tokens.current_n,
                report.tiles.tokens.previous_n
            ),
            (cost.current_n, cost.previous_n)
        );
    }

    #[test]
    fn dashboard_f9_names_hands_off_timestamp_health_exclusions() {
        let f = fixture("F9");
        let data = &f.snapshots()["hands_off"];
        for case in data["cases"].as_array().unwrap() {
            let mut db = TempDb::empty().unwrap();
            for name in case["sessions"].as_array().unwrap() {
                let session = data["sessions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|s| s["session_id"] == *name)
                    .unwrap();
                seed(
                    db.store_mut(),
                    name.as_str().unwrap(),
                    "claude",
                    Some("raw-batched"),
                    &records(&session["records"]),
                );
            }
            let report = report(&db, 7);
            let label = case["name"].as_str().unwrap();
            let tile = &report.tiles.hands_off_median;
            assert_eq!(tile.current_n, case["expected_n"].as_u64(), "{label}");
            if case["excluded"].as_bool().unwrap() {
                assert_eq!(
                    report.hands_off_excluded_surfaces,
                    [MetricExcludedSurface {
                        host: "claude".into(),
                        surface: Some("raw-batched".into()),
                        qualifying_sessions: case["qualifying_sessions"].as_u64().unwrap(),
                        degenerate_sessions: case["degenerate_sessions"].as_u64().unwrap(),
                    }],
                    "{label}"
                );
                assert_eq!(tile.value, None, "{label}");
                assert!(tile.reason.is_some(), "{label}");
            } else {
                assert!(report.hands_off_excluded_surfaces.is_empty(), "{label}");
            }
        }
    }

    #[test]
    fn dashboard_daily_hours_split_the_hero_totals_without_re_measuring_a_day() {
        let mut db = TempDb::empty().unwrap();
        // One stretch of work either side of local midnight: a fifteen-minute
        // agent span, and a human message on the later date whose estimate
        // reaches back to the agent event on the earlier one. Whatever range is
        // selected, the days must add up to the hero's own totals.
        seed(
            db.store_mut(),
            "crossing",
            "claude",
            Some("cli"),
            &records(&json!([
                {"uuid":"a1","type":"assistant","timestamp":"2026-09-05T23:50:00Z",
                 "message":{"role":"assistant","model":"test-claude","content":[]}},
                {"uuid":"h1","type":"user","timestamp":"2026-09-06T00:05:00Z",
                 "message":{"role":"user","content":[{"type":"text","text":"x"}]}},
            ])),
        );
        for days in [7, 14, 30] {
            let report = report(&db, days);
            assert_eq!(report.days.len(), days as usize);
            assert_eq!(report.tiles.agent_hours.value, Some(0.25));
            assert_eq!(report.tiles.human_hours_est.value, Some(1.0 / 12000.0));
            let agent: f64 = report.days.iter().map(|day| day.agent_hours).sum();
            let human: f64 = report
                .days
                .iter()
                .map(|day| day.human_hours_est.unwrap())
                .sum();
            assert!((agent - 0.25).abs() < 1e-12, "{agent}");
            assert!((human - 1.0 / 12000.0).abs() < 1e-12, "{human}");
            // Ten minutes before midnight and five after, for both measurements:
            // a day re-measured on its own would have neither the span's earlier
            // endpoint nor the human estimate's previous agent event.
            let worked: Vec<_> = report
                .days
                .iter()
                .filter(|day| day.agent_hours > 0.0)
                .map(|day| (day.date.as_str(), day.agent_hours, day.human_hours_est))
                .collect();
            assert_eq!(
                worked,
                [
                    ("2026-09-05", 10.0 / 60.0, Some(0.0)),
                    ("2026-09-06", 5.0 / 60.0, Some(1.0 / 12000.0)),
                ]
            );
        }
    }

    #[test]
    fn dashboard_daily_human_hours_are_unknown_together_while_agent_stays_measured() {
        let mut db = TempDb::empty().unwrap();
        // A user record whose content was never observed cannot be classified,
        // which leaves this window's human estimate unknown as a whole: no day
        // may report a human zero, and agent time is unaffected.
        seed(
            db.store_mut(),
            "mixed",
            "claude",
            Some("cli"),
            &records(&json!([
                {"uuid":"a1","type":"assistant","timestamp":"2026-09-06T09:00:00Z",
                 "message":{"role":"assistant","model":"test-claude","content":[]}},
                {"uuid":"u1","type":"user","timestamp":"2026-09-06T09:15:00Z",
                 "message":{"role":"user"}},
                {"uuid":"a2","type":"assistant","timestamp":"2026-09-06T09:30:00Z",
                 "message":{"role":"assistant","model":"test-claude","content":[]}},
            ])),
        );
        let report = report(&db, 7);
        assert_eq!(report.tiles.human_hours_est.value, None);
        assert!(report.tiles.human_hours_est.reason.is_some());
        assert_eq!(report.tiles.agent_hours.value, Some(0.5));
        assert!(report.days.iter().all(|day| day.human_hours_est.is_none()));
        assert_eq!(
            report.days.iter().map(|day| day.agent_hours).sum::<f64>(),
            0.5
        );
        // An unknown human day serializes as null, never as a measured zero.
        let json = serde_json::to_value(&report).unwrap();
        for day in json["days"].as_array().unwrap() {
            assert_eq!(day["human_hours_est"], serde_json::Value::Null);
        }
    }

    #[test]
    fn dashboard_agent_hours_per_day_counts_partial_and_dst_local_days() {
        let mut db = TempDb::empty().unwrap();
        seed(
            db.store_mut(),
            "dst",
            "claude",
            Some("cli"),
            &[
                assistant("dst-a", "2026-11-01T05:30:00Z", None),
                assistant("dst-b", "2026-11-01T05:50:00Z", None),
                assistant("dst-c", "2026-11-01T06:10:00Z", None),
                assistant("dst-d", "2026-11-01T06:30:00Z", None),
            ],
        );
        let zone = TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();
        // Local 10:00 on the third: first and final local dates are partial, and
        // the 25-hour transition date still contributes one bucket.
        let now = ms("2026-11-03T15:00:00Z");
        let metrics = MetricsDb::open(db.path()).unwrap();
        let report = assemble(
            &metrics,
            7,
            now,
            zone,
            MetricClock::System,
            &catalog(),
            TypingRate::default(),
        )
        .unwrap();
        assert_eq!(report.days.len(), 8);
        assert_eq!(report.days[0].date, "2026-10-27");
        assert_eq!(report.days[7].date, "2026-11-03");
        let transition = report.days.iter().find(|d| d.date == "2026-11-01").unwrap();
        assert_eq!(transition.end_ms - transition.start_ms, 25 * 3_600_000);
        assert_eq!(report.tiles.agent_hours.value, Some(1.0));
        assert_eq!(report.tiles.agent_hours_per_day.value, Some(1.0 / 8.0));
        assert_eq!(report.tiles.sessions_per_day.value, Some(1.0 / 8.0));
        // The hour is local to the 25-hour transition date and stays whole in
        // its one bucket; the partial first and last dates are measured zeros.
        assert_eq!(transition.agent_hours, 1.0);
        assert_eq!(
            report.days.iter().map(|day| day.agent_hours).sum::<f64>(),
            1.0
        );
        assert!(
            report
                .days
                .iter()
                .filter(|day| day.date != "2026-11-01")
                .all(|day| day.agent_hours == 0.0)
        );
        assert!(
            report
                .days
                .iter()
                .all(|day| day.human_hours_est == Some(0.0))
        );
        let note = report.tiles.agent_hours_per_day.note.unwrap();
        assert!(note.contains("divided by 8 reported local date buckets"));
        assert!(note.contains("previous denominator: 8"));
        assert_eq!(report.window.clock, MetricClock::System);
    }

    #[test]
    fn dashboard_rejects_unsafe_json_integers_and_unsupported_ranges() {
        let mut db = TempDb::empty().unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        for days in [0, 1, 13, 15, 31] {
            assert!(matches!(
                assemble(
                    &metrics,
                    days,
                    now(),
                    TimeZone::UTC,
                    MetricClock::Fixture,
                    &catalog(),
                    TypingRate::default()
                ),
                Err(StateError::InvalidMetricWindow)
            ));
        }
        drop(metrics);
        let huge = 1_i64 << 53;
        seed(
            db.store_mut(),
            "huge",
            "claude",
            Some("cli"),
            &[assistant(
                "huge",
                "2026-09-07T12:00:00Z",
                Some(json!({"input_tokens":huge,"output_tokens":0,
                    "cache_read_input_tokens":0,"cache_creation_input_tokens":0})),
            )],
        );
        let metrics = MetricsDb::open(db.path()).unwrap();
        assert!(matches!(
            assemble(
                &metrics,
                7,
                now(),
                TimeZone::UTC,
                MetricClock::Fixture,
                &catalog(),
                TypingRate::default()
            ),
            Err(StateError::CountRange)
        ));
    }

    #[test]
    fn dashboard_fixture_state_is_isolated_pinned_and_path_free() {
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
        assert_eq!(state.database_path(), None);
        assert_eq!(state.native_home(), None);
        let data_dir = state.app_info().data_dir;
        let manifest_now = fixture("F1").now().timestamp_millis();
        for days in [7, 14, 30] {
            let report = state.metrics_dashboard(days).unwrap();
            assert_eq!(report.window.end_ms, manifest_now);
            assert_eq!(
                (report.window.timezone.as_str(), report.window.clock.clone()),
                ("UTC", MetricClock::Fixture)
            );
            let text = serde_json::to_string(&report).unwrap();
            assert!(!text.contains(&data_dir) && !text.contains("xtrace.db"));
            let hosts = state.tokens_by_host(days).unwrap();
            assert_eq!(hosts.window, report.window);
            assert_eq!(hosts.hosts, report.tokens_by_host);
        }
        // Fixture F1 records no service tier: legitimately unpriced.
        let report = state.metrics_dashboard(7).unwrap();
        assert_eq!(report.cost.total_usd, None);
        assert!(
            report
                .cost
                .unpriced
                .iter()
                .all(|u| u.reason == MetricUnpricedReason::MissingServiceTier)
        );
        assert!(matches!(
            state.metrics_dashboard(15),
            Err(StateError::InvalidMetricWindow)
        ));
        state.shutdown();
        assert!(!std::path::Path::new(&data_dir).exists());
        assert!(matches!(
            state.metrics_dashboard(7),
            Err(StateError::Closed)
        ));
        assert!(matches!(state.tokens_by_host(7), Err(StateError::Closed)));
    }

    /// The four canonical counters, all measured, with an explicit output.
    fn counters(output: u64) -> serde_json::Value {
        json!({"input_tokens":10,"output_tokens":output,
            "cache_read_input_tokens":0,"cache_creation_input_tokens":0})
    }
    /// The four counters at the catalog's standard tier, so the response is
    /// priced: 10 input and `output` output tokens of `test-claude` cost
    /// 10 × 1,000 + output × 10,000 nano-USD.
    fn priced(output: u64) -> serde_json::Value {
        json!({"input_tokens":10,"output_tokens":output,
            "cache_read_input_tokens":0,"cache_creation_input_tokens":0,
            "service_tier":"standard"})
    }
    /// One Claude response snapshot, addressable by the (message id, request
    /// id) pair the M-04 selection groups on.
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
    /// A session carrying the stored context a lane row names it by.
    fn seed_context(
        store: &mut Store,
        id: &str,
        cwd: Option<&str>,
        branch: Option<&str>,
        rows: &[CanonicalRecord],
    ) {
        let mut session = SessionMeta::new(id, "claude", SessionSource::Fixture);
        session.surface = Some("cli".into());
        session.cwd = cwd.map(str::to_owned);
        session.git_branch = branch.map(str::to_owned);
        store.upsert_session(&session, false).unwrap();
        store.upsert_records(id, rows, false).unwrap();
    }
    fn lane_session<'a>(
        report: &'a DashboardMetrics,
        id: &str,
    ) -> Option<&'a DashboardLaneSession> {
        report.lane_sessions.iter().find(|s| s.session_id == id)
    }

    #[test]
    fn dashboard_lane_sessions_name_indexed_context_over_the_fixed_window() {
        let mut db = TempDb::empty().unwrap();
        seed_context(
            db.store_mut(),
            "ctx-repo",
            Some("/Users/dev/code/acme-api"),
            Some("feature/lanes"),
            &[
                response("ctx-repo-a", "2026-09-07T10:00:00Z", "r1", priced(7)),
                response("ctx-repo-b", "2026-09-07T10:05:00Z", "r2", priced(7)),
                // Inside the selected range but outside the fixed lane window.
                response("ctx-repo-old", "2026-09-04T10:00:00Z", "r3", counters(100)),
            ],
        );
        // An indexed session whose history carries neither repository nor
        // branch, and measures no usage at all.
        seed_context(
            db.store_mut(),
            "ctx-bare",
            None,
            None,
            &[assistant("ctx-bare-a", "2026-09-07T11:00:00Z", None)],
        );
        let report = report(&db, 7);
        // One row per distinct session the returned spans name, by identifier.
        assert_eq!(report.lanes_total, 2);
        let named: Vec<&str> = report
            .lane_sessions
            .iter()
            .map(|s| s.session_id.as_str())
            .collect();
        assert_eq!(named, ["ctx-bare", "ctx-repo"]);
        let repo = lane_session(&report, "ctx-repo").unwrap();
        assert_eq!(
            (
                repo.host.as_str(),
                repo.repo.as_deref(),
                repo.branch.as_deref()
            ),
            (
                "claude",
                Some("/Users/dev/code/acme-api"),
                Some("feature/lanes")
            )
        );
        // The lane window only: the older response is not in these 48 hours.
        let cost = repo.cost.as_ref().unwrap();
        assert_eq!(
            (cost.selected_observations, cost.priced_observations),
            (2, 2)
        );
        assert_eq!(cost.total_usd, Some(0.00016));
        let bare = lane_session(&report, "ctx-bare").unwrap();
        assert_eq!((bare.repo.as_deref(), bare.branch.as_deref()), (None, None));
        // Indexed, but no selected response: nothing to price, never a zero.
        let cost = bare.cost.as_ref().unwrap();
        assert_eq!((cost.selected_observations, cost.total_usd), (0, None));
        // The selected range's own totals are untouched by the lane reads.
        assert_eq!(report.tokens.counters.output_tokens, Some(114));
        assert_eq!(report.tokens.counters.total_tokens, Some(144));
        assert_eq!(report.tiles.sessions.value, Some(2.0));
    }

    #[test]
    fn dashboard_lane_cost_separates_zero_partial_unpriced_and_empty() {
        let mut db = TempDb::empty().unwrap();
        seed_context(
            db.store_mut(),
            "zero-cost",
            None,
            None,
            &[response(
                "zero-a",
                "2026-09-07T12:00:00Z",
                "z1",
                json!({"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,
                    "cache_creation_input_tokens":0,"service_tier":"standard"}),
            )],
        );
        // One priced response beside one that records no service tier.
        seed_context(
            db.store_mut(),
            "partial-cost",
            None,
            None,
            &[
                response("partial-a", "2026-09-07T12:00:00Z", "p1", priced(1)),
                response("partial-b", "2026-09-07T12:01:00Z", "p2", counters(5)),
            ],
        );
        seed_context(
            db.store_mut(),
            "unpriced-cost",
            None,
            None,
            &[response(
                "unpriced-a",
                "2026-09-07T12:00:00Z",
                "u1",
                counters(5),
            )],
        );
        seed_context(
            db.store_mut(),
            "no-usage",
            None,
            None,
            &[assistant("none-a", "2026-09-07T12:00:00Z", None)],
        );
        let report = report(&db, 7);
        let cost = |id: &str| lane_session(&report, id).unwrap().cost.clone().unwrap();
        // A measured zero is a zero.
        let zero = cost("zero-cost");
        assert_eq!((zero.priced_observations, zero.total_usd), (1, Some(0.0)));
        // Partial: a priced subtotal, no total, and the unpriced response named.
        let partial = cost("partial-cost");
        assert_eq!(
            (
                partial.selected_observations,
                partial.priced_observations,
                partial.unpriced_observations,
                partial.total_usd,
                partial.priced_subtotal_usd,
            ),
            (2, 1, 1, None, 0.00002)
        );
        // The unpriced response is named by model, tier and reason.
        assert_eq!(
            partial.unpriced,
            [DashboardUnpriced {
                model: Some("test-claude".into()),
                service_tier: None,
                reason: MetricUnpricedReason::MissingServiceTier,
                observations: 1,
            }]
        );
        let unpriced = cost("unpriced-cost");
        assert_eq!(
            (
                unpriced.priced_observations,
                unpriced.unpriced_observations,
                unpriced.priced_subtotal_usd
            ),
            (0, 1, 0.0)
        );
        let empty = cost("no-usage");
        assert_eq!((empty.selected_observations, empty.total_usd), (0, None));
        // Every response here is inside both windows, so the lane costs are a
        // slice of the range's own cost report: the same selection and prices.
        // The DTO carries dollars as f64, so the sum is compared to within a
        // thousandth of a nano-dollar (the pricer's own unit is the nano-dollar;
        // xt-metrics checks the same sum exactly in nano-USD).
        let lanes: f64 = report
            .lane_sessions
            .iter()
            .map(|s| s.cost.as_ref().unwrap().priced_subtotal_usd)
            .sum();
        assert!(
            (lanes - report.cost.priced_subtotal_usd).abs() < 1e-12,
            "{lanes} != {}",
            report.cost.priced_subtotal_usd
        );
        assert_eq!(
            report
                .lane_sessions
                .iter()
                .map(|s| s.cost.as_ref().unwrap().selected_observations)
                .sum::<u64>(),
            report.cost.selected_observations
        );
    }

    #[test]
    fn dashboard_lane_session_dedupes_copied_responses_across_its_spans() {
        let mut db = TempDb::empty().unwrap();
        seed_context(
            db.store_mut(),
            "split",
            Some("/Users/dev/code/acme-api"),
            None,
            &[
                // Two snapshots of one response: the latest is selected once.
                response("split-a", "2026-09-07T12:00:00Z", "req-1", priced(5)),
                response("split-b", "2026-09-07T12:00:01Z", "req-1", priced(9)),
                // More than twenty minutes later: a second span, same session.
                response("split-c", "2026-09-07T13:30:00Z", "req-2", priced(3)),
            ],
        );
        let report = report(&db, 7);
        let spans: Vec<_> = report
            .lanes
            .iter()
            .filter(|lane| lane.session_id == "split")
            .collect();
        assert_eq!(spans.len(), 2);
        // Several spans, one context row, and the response counted once.
        assert_eq!(report.lane_sessions.len(), 1);
        let split = lane_session(&report, "split").unwrap();
        let cost = split.cost.as_ref().unwrap();
        assert_eq!(cost.selected_observations, 2);
        assert_eq!(cost.total_usd, Some(0.00014));
        assert_eq!(split.repo.as_deref(), Some("/Users/dev/code/acme-api"));
        assert_eq!(split.branch, None);
    }

    #[test]
    fn dashboard_lane_session_cost_survives_the_display_cap() {
        let mut db = TempDb::empty().unwrap();
        // Fillers newer than the capped session's earlier span, so the cap
        // drops that span while keeping its most recent one.
        for index in 0..LANE_LIMIT {
            let id = format!("filler-{index:03}");
            seed_context(
                db.store_mut(),
                &id,
                None,
                None,
                &[assistant(&format!("{id}-a"), "2026-09-07T22:00:00Z", None)],
            );
        }
        seed_context(
            db.store_mut(),
            "capped",
            Some("/Users/dev/code/acme-api"),
            Some("main"),
            &[
                response("capped-old", "2026-09-06T04:00:00Z", "c1", priced(40)),
                response("capped-new", "2026-09-07T23:30:00Z", "c2", priced(2)),
            ],
        );
        let report = report(&db, 7);
        assert!(report.lanes_truncated);
        assert_eq!(report.lanes.len(), LANE_LIMIT);
        assert_eq!(report.lanes_total, LANE_LIMIT as u64 + 2);
        // Only the newest span of the capped session is drawn.
        assert_eq!(
            report
                .lanes
                .iter()
                .filter(|lane| lane.session_id == "capped")
                .count(),
            1
        );
        // Context rows describe exactly the sessions the returned spans name.
        let named: std::collections::BTreeSet<&str> = report
            .lane_sessions
            .iter()
            .map(|s| s.session_id.as_str())
            .collect();
        let drawn: std::collections::BTreeSet<&str> = report
            .lanes
            .iter()
            .map(|lane| lane.session_id.as_str())
            .collect();
        assert_eq!(named, drawn);
        assert_eq!(report.lane_sessions.len(), LANE_LIMIT);
        // The measurement still covers the whole window, including the span
        // the cap left out.
        let cost = lane_session(&report, "capped")
            .unwrap()
            .cost
            .clone()
            .unwrap();
        assert_eq!(cost.selected_observations, 2);
        assert_eq!(cost.total_usd, Some(0.00044));
    }

    #[test]
    fn dashboard_lane_cap_keeps_the_native_start_of_a_capped_session() {
        let mut db = TempDb::empty().unwrap();
        for index in 0..LANE_LIMIT {
            let id = format!("filler-{index:03}");
            seed_context(
                db.store_mut(),
                &id,
                None,
                None,
                &[assistant(&format!("{id}-a"), "2026-09-07T22:00:00Z", None)],
            );
        }
        let mut capped = SessionMeta::new("capped", "claude", SessionSource::Fixture);
        capped.started_at_ms = Some(ms("2026-09-06T03:00:00Z"));
        db.store_mut().upsert_session(&capped, false).unwrap();
        db.store_mut()
            .upsert_records(
                "capped",
                &[
                    response("capped-old", "2026-09-06T04:00:00Z", "c1", counters(40)),
                    response("capped-new", "2026-09-07T23:30:00Z", "c2", counters(2)),
                ],
                false,
            )
            .unwrap();
        pr_link(db.store_mut(), "capped", 5, PrConfidence::Exact, 1);
        let report = report(&db, 7);
        assert!(report.lanes_truncated);
        let drawn: Vec<_> = report
            .lanes
            .iter()
            .filter(|lane| lane.session_id == "capped")
            .collect();
        assert_eq!(drawn.len(), 1);
        let row = lane_session(&report, "capped").unwrap();
        // Neither the cap nor the drawn span moves the recorded start.
        assert_eq!(row.started_at_ms, Some(ms("2026-09-06T03:00:00Z")));
        assert!(row.started_at_ms.unwrap() < drawn[0].start_ms);
        assert_eq!(
            (
                row.pr_links,
                row.cost.as_ref().map(|c| c.selected_observations)
            ),
            (Some(1), Some(2))
        );
    }

    fn pr_link(store: &mut Store, id: &str, number: u64, confidence: PrConfidence, seen: i64) {
        store
            .record_pr_link(&PrLinkObservation {
                session_id: id.into(),
                pull_request: PrIdentity::from_parts("example/atlas", number).unwrap(),
                confidence,
                first_seen_at: seen,
                last_seen_at: seen,
            })
            .unwrap();
    }

    #[test]
    fn dashboard_lane_sessions_carry_native_start_saved_title_and_recorded_links() {
        let mut db = TempDb::empty().unwrap();
        db.store_mut()
            .set_retention_mode(xt_store::retention::RetentionMode::FullContent)
            .unwrap();
        // A native start long before the 48-hour axis and the selected range.
        let mut old = SessionMeta::new("started-old", "claude", SessionSource::Fixture);
        old.started_at_ms = Some(ms("2026-06-01T08:30:00Z"));
        old.title = Some("Saved title".into());
        db.store_mut().upsert_session(&old, true).unwrap();
        db.store_mut()
            .upsert_records(
                "started-old",
                &[response("old-a", "2026-09-07T10:00:00Z", "o1", counters(4))],
                false,
            )
            .unwrap();
        // Valid events in the window and a blank saved title. Claude Code
        // records no start, so its start is its earliest message.
        let mut untimed = SessionMeta::new("start-unknown", "claude", SessionSource::Fixture);
        untimed.title = Some("   ".into());
        db.store_mut().upsert_session(&untimed, true).unwrap();
        db.store_mut()
            .upsert_records(
                "start-unknown",
                &[assistant("unknown-a", "2026-09-07T11:00:00Z", None)],
                false,
            )
            .unwrap();
        // Another host that recorded no start: its start stays unknown.
        let codex = SessionMeta::new("codex-start-unknown", "codex", SessionSource::Fixture);
        db.store_mut().upsert_session(&codex, false).unwrap();
        db.store_mut()
            .upsert_records(
                "codex-start-unknown",
                &[assistant("codex-a", "2026-09-07T11:30:00Z", None)],
                false,
            )
            .unwrap();
        // Indexed, in the window, never linked.
        seed_context(
            db.store_mut(),
            "unlinked",
            None,
            None,
            &[assistant("unlinked-a", "2026-09-07T12:00:00Z", None)],
        );
        let before: Vec<_> = [7, 30].map(|days| report(&db, days)).into();
        {
            let store = db.store_mut();
            // Exact, sha and inferred evidence; several PRs on one session; one
            // PR shared with another session; a link observed long before any
            // window.
            pr_link(store, "started-old", 1, PrConfidence::Exact, 1);
            pr_link(
                store,
                "started-old",
                2,
                PrConfidence::Sha,
                ms("2026-09-07T10:00:00Z"),
            );
            pr_link(store, "started-old", 3, PrConfidence::Inferred, 1);
            pr_link(store, "start-unknown", 1, PrConfidence::Inferred, 1);
            // Merge and refresh state never change what is counted: one PR is
            // refreshed as merged, one as open, the third never refreshed.
            for (number, state) in [(1, PrState::Merged), (2, PrState::Open)] {
                store
                    .record_pr_refresh(&RefreshOutcome::Success(RefreshSuccess {
                        pull_request: PrIdentity::from_parts("example/atlas", number).unwrap(),
                        attempted_at: 100,
                        title: format!("Synthetic {number}"),
                        state,
                        merged_at: (state == PrState::Merged)
                            .then(|| "2026-09-07T09:00:00Z".into()),
                        additions: 1,
                        deletions: 1,
                        head_ref_name: "feature/synthetic".into(),
                    }))
                    .unwrap();
            }
        }
        for (days, before) in [7, 30].into_iter().zip(before) {
            let report = report(&db, days);
            let old = lane_session(&report, "started-old").unwrap();
            // The stored start, not the first span inside the window.
            assert_eq!(old.started_at_ms, Some(ms("2026-06-01T08:30:00Z")));
            assert!(old.started_at_ms.unwrap() < report.lane_start_ms);
            assert_eq!(old.title.as_deref(), Some("Saved title"));
            assert_eq!((old.pr_links, old.inferred_pr_links), (Some(3), Some(1)));
            let unknown = lane_session(&report, "start-unknown").unwrap();
            assert_eq!(
                (unknown.started_at_ms, unknown.title.as_deref()),
                (Some(ms("2026-09-07T11:00:00Z")), None)
            );
            let codex = lane_session(&report, "codex-start-unknown").unwrap();
            assert_eq!(codex.started_at_ms, None);
            assert_eq!(
                (unknown.pr_links, unknown.inferred_pr_links),
                (Some(1), Some(1))
            );
            let unlinked = lane_session(&report, "unlinked").unwrap();
            // Indexed with no link: a measured zero, not unknown.
            assert_eq!(
                (unlinked.pr_links, unlinked.inferred_pr_links),
                (Some(0), Some(0))
            );
            // Adding links changes no row, span, total or session tile: only
            // the counts. The merged-PR tile legitimately counts the linked PR
            // refreshed as merged (M-19), so it alone is compared apart.
            assert_eq!(report.lanes, before.lanes);
            let mut tiles = report.tiles.clone();
            tiles.merged_prs = before.tiles.merged_prs.clone();
            assert_eq!(tiles, before.tiles);
            assert_eq!(report.tiles.merged_prs.value, Some(1.0));
            assert_eq!(report.tokens, before.tokens);
            assert_eq!(
                report
                    .lane_sessions
                    .iter()
                    .map(|s| (&s.session_id, s.cost.clone(), s.started_at_ms))
                    .collect::<Vec<_>>(),
                before
                    .lane_sessions
                    .iter()
                    .map(|s| (&s.session_id, s.cost.clone(), s.started_at_ms))
                    .collect::<Vec<_>>()
            );
        }
        // The selected range never filters the counts or the start.
        let (week, month) = (report(&db, 7), report(&db, 30));
        assert_eq!(week.lane_sessions, month.lane_sessions);
    }

    /// Lane context and lane costs are read inside the one dashboard
    /// snapshot, by construction: both reads happen only in `lane_report`, and
    /// `lane_report` is called only from inside `read_snapshot`'s closure. The
    /// spans are read only by `lane_spans`, which `lane_report` calls; the
    /// fixture span-detail export calls it too, for spans alone.
    #[test]
    fn dashboard_lane_reads_stay_inside_one_snapshot_by_construction() {
        let module = include_str!("../src/dashboard.rs");
        let body_of = |name: &str| {
            let (_, after) = module.split_once(name).expect(name);
            after.split_once("\n}\n").expect("body").0
        };
        let body = body_of("fn lane_report(");
        for read in [
            "db.session_context(",
            "db.session_costs(",
            "lane_spans(db, lane_window)",
        ] {
            assert_eq!(module.matches(read).count(), 1, "{read} is read elsewhere");
            assert!(body.contains(read), "{read} is not read by lane_report");
        }
        let read = "db.active_spans(lane_window)";
        assert_eq!(module.matches(read).count(), 1, "{read} is read elsewhere");
        assert!(body_of("fn lane_spans(").contains(read));
        let (_, assembled) = module.split_once("pub fn assemble(").expect("assemble");
        let (snapshot, _) = assembled
            .split_once("    })?;")
            .expect("assemble read snapshot");
        let (_, closure) = snapshot
            .split_once("db.read_snapshot(|db| {")
            .expect("one snapshot");
        assert!(closure.contains("lane_report(db, lane_window, catalog)?"));
        assert_eq!(module.matches("lane_report(").count(), 2);
    }
}

fn native_state(root: &std::path::Path) -> AppState {
    std::fs::create_dir_all(root.join("home")).unwrap();
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
fn dashboard_native_state_rereads_inserts_enrichment_and_receipts() {
    use xt_store::{
        Host,
        ingest::{CaptureReceipt, DiscoveredSession, RecordCoverage},
    };
    let root = tempfile::TempDir::new().unwrap();
    let state = native_state(root.path());
    let empty = state.metrics_dashboard(7).unwrap();
    assert_eq!(empty.window.clock, xtrace_desktop::dto::MetricClock::System);
    assert_eq!(empty.tiles.sessions.value, Some(0.0));

    let now = Timestamp::now().as_second();
    let at = |offset: i64| Timestamp::from_second(now + offset).unwrap().to_string();
    let mut writer = Store::open(root.path().join("data/xtrace.db")).unwrap();
    // Insertion.
    seed(
        &mut writer,
        "live",
        "claude",
        Some("cli"),
        &[
            assistant("live-a", &at(-3600), None),
            assistant("live-b", &at(-3000), None),
        ],
    );
    let inserted = state.metrics_dashboard(7).unwrap();
    assert_eq!(inserted.tiles.sessions.value, Some(1.0));
    assert_eq!(inserted.tiles.agent_hours.value, Some(600.0 / 3600.0));
    assert_eq!(inserted.lanes_total, 1);
    assert_eq!(inserted.tiles.tokens.value, None);
    let records = || serde_json::to_value(state.db_counts().unwrap()).unwrap()["records"].clone();
    let inserted_records = records();

    // Enrichment of an existing UUID: no new record, newly measured usage.
    seed(
        &mut writer,
        "live",
        "claude",
        Some("cli"),
        &[assistant(
            "live-b",
            &at(-3000),
            Some(json!({"input_tokens":10,"output_tokens":5,
                "cache_read_input_tokens":0,"cache_creation_input_tokens":0})),
        )],
    );
    assert_eq!(records(), inserted_records);
    let enriched = state.metrics_dashboard(7).unwrap();
    assert_eq!(enriched.tiles.tokens.value, Some(15.0));
    assert_eq!(enriched.usage_coverage.total.measured, 1);
    let hosts = state.tokens_by_host(7).unwrap();
    assert_eq!(hosts.hosts, enriched.tokens_by_host);
    assert_eq!(hosts.hosts[0].tokens.counters.total_tokens, Some(15));

    // A discovered session, then its sealed capture receipt.
    let now_ms = now * 1000;
    writer
        .observe_discovered_session(&DiscoveredSession {
            host: Host::Claude,
            native_session_id: "live".into(),
            conversation_id: Some("live".into()),
            surface: Some("cli".into()),
            started_at_ms: Some(now_ms - 3_600_000),
            last_observed_at: now_ms,
            discovery_complete: true,
        })
        .unwrap();
    let discovered = state.metrics_dashboard(7).unwrap();
    assert_eq!(discovered.capture_coverage.len(), 1);
    assert_eq!(
        (
            discovered.capture_coverage[0].observed_sessions,
            discovered.capture_coverage[0].captured_sessions
        ),
        (1, 0)
    );
    writer
        .insert_capture_receipt(
            &CaptureReceipt {
                receipt_id: "receipt".into(),
                session_id: "live".into(),
                surface: Some("cli".into()),
                received_at: now_ms,
            },
            &[RecordCoverage {
                record_uuid: "live-b".into(),
                metric_field_mask: 1,
                measurement_revision: "a".repeat(64),
                digest_schema_version: 1,
            }],
        )
        .unwrap();
    let captured = state.metrics_dashboard(7).unwrap();
    assert_eq!(captured.capture_coverage[0].captured_sessions, 1);
    // Receipt history is not a complete runtime inventory.
    assert_eq!(
        captured.capture_inventory,
        xtrace_desktop::dto::MetricInventory::Unknown
    );
    assert_eq!(
        captured.capture_coverage[0].inventory,
        xtrace_desktop::dto::MetricInventory::Unknown
    );
}

#[test]
fn dashboard_native_state_validates_ranges_and_closes() {
    let root = tempfile::TempDir::new().unwrap();
    let state = native_state(root.path());
    for days in [7, 14, 30] {
        assert_eq!(state.metrics_dashboard(days).unwrap().window.days, days);
        assert_eq!(state.tokens_by_host(days).unwrap().window.days, days);
    }
    for days in [0, 15, 90] {
        assert!(matches!(
            state.metrics_dashboard(days),
            Err(StateError::InvalidMetricWindow)
        ));
        assert!(matches!(
            state.tokens_by_host(days),
            Err(StateError::InvalidMetricWindow)
        ));
    }
    state.shutdown();
    assert!(matches!(
        state.metrics_dashboard(7),
        Err(StateError::Closed)
    ));
    assert!(matches!(state.tokens_by_host(7), Err(StateError::Closed)));
}
