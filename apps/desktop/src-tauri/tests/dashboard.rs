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
    use xt_metrics::{MetricsDb, PriceCatalog};
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
        for tile in [&report.tiles.merged_prs, &report.tiles.rule_fires] {
            assert_eq!(tile.value, None);
            assert!(tile.reason.is_some());
            assert_eq!(tile.sample_unit, "unavailable");
            assert!(tile.delta.suppressed);
        }
        let keys: Vec<_> = report.unavailable.iter().map(|u| u.key.as_str()).collect();
        assert_eq!(
            keys,
            ["merged_prs", "rule_fires", "environment", "work_type"]
        );
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
        assert!(report.tokens_by_host.is_empty());
        assert_eq!(report.days.len(), 30);
        assert!(report.days.iter().all(|day| day.sessions == 0));
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
        let report = assemble(&metrics, 7, now, zone, MetricClock::System, &catalog()).unwrap();
        assert_eq!(report.days.len(), 8);
        assert_eq!(report.days[0].date, "2026-10-27");
        assert_eq!(report.days[7].date, "2026-11-03");
        let transition = report.days.iter().find(|d| d.date == "2026-11-01").unwrap();
        assert_eq!(transition.end_ms - transition.start_ms, 25 * 3_600_000);
        assert_eq!(report.tiles.agent_hours.value, Some(1.0));
        assert_eq!(report.tiles.agent_hours_per_day.value, Some(1.0 / 8.0));
        assert_eq!(report.tiles.sessions_per_day.value, Some(1.0 / 8.0));
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
                    &catalog()
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
                &catalog()
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
