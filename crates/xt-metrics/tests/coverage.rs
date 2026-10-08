use jiff::Timestamp;
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{
    CaptureGap, Coverage, DiscoveryHealth, InventoryState, MetricsDb, PriceCatalog, UsageGap,
    Window,
};
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource,
    ingest::{CaptureReceipt, DiscoveredSession, RecordCoverage},
};
fn fixture(id: &str) -> Fixture {
    Fixture::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(id),
    )
    .unwrap()
}
mod report_support;
fn ms(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn health(host: &str, surface: Option<&str>, inventory: InventoryState) -> DiscoveryHealth {
    DiscoveryHealth {
        host: host.into(),
        surface: surface.map(str::to_owned),
        inventory,
    }
}
fn query(db: &TempDb, health: &[DiscoveryHealth]) -> Coverage {
    coverage_in(
        &MetricsDb::open(db.path()).unwrap(),
        window(),
        window().end_ms(),
        health,
    )
    .unwrap()
}
/// The coverage report, checked against the combined report read.
fn coverage_in(
    metrics: &MetricsDb,
    window: Window,
    now_ms: i64,
    health: &[DiscoveryHealth],
) -> xt_metrics::Result<Coverage> {
    let catalog = PriceCatalog::bundled().unwrap();
    report_support::matches_standalone(
        metrics,
        window,
        now_ms,
        health,
        jiff::tz::TimeZone::UTC,
        &catalog,
    )
    .coverage
}
/// The trailing 14 days ending at the window's end: the gate's own window.
fn fourteen_days() -> Window {
    Window::new(window().end_ms() - 14 * 86_400_000, window().end_ms()).unwrap()
}
fn seed(db: &mut TempDb, id: &str, host: &str, surface: Option<&str>, rows: &[CanonicalRecord]) {
    let mut session = SessionMeta::new(id, host, SessionSource::Fixture);
    session.surface = surface.map(str::to_owned);
    db.store_mut().upsert_session(&session, false).unwrap();
    db.store_mut().upsert_records(id, rows, false).unwrap();
}
fn record(id: &str, measured: bool) -> CanonicalRecord {
    let mut v = json!({"uuid":id,"type":"assistant","timestamp":"2026-09-07T12:00:00Z","message":{"role":"assistant","model":"test-model","content":[{"type":"text","text":"Synthetic"}]}});
    if measured {
        v["message"]["usage"] = json!({"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0});
    }
    serde_json::from_value(v).unwrap()
}
fn discover(
    db: &mut TempDb,
    native: &str,
    conversation: Option<&str>,
    host: Host,
    surface: Option<&str>,
    start: Option<i64>,
) {
    db.store_mut()
        .observe_discovered_session(&DiscoveredSession {
            host,
            native_session_id: native.into(),
            conversation_id: conversation.map(str::to_owned),
            surface: surface.map(str::to_owned),
            started_at_ms: start,
            last_observed_at: window().end_ms(),
            discovery_complete: true,
        })
        .unwrap();
}
fn receipt(db: &mut TempDb, id: &str, session: &str, surface: Option<&str>, uuid: &str) {
    db.store_mut()
        .insert_capture_receipt(
            &CaptureReceipt {
                receipt_id: id.into(),
                session_id: session.into(),
                surface: surface.map(str::to_owned),
                received_at: window().end_ms(),
            },
            &[RecordCoverage {
                record_uuid: uuid.into(),
                metric_field_mask: 1,
                measurement_revision: "a".repeat(64),
                digest_schema_version: 1,
            }],
        )
        .unwrap();
}

#[test]
fn coverage_usage_real_f6_f7_f10_zero_partial_blank_and_absent() {
    let mut db = TempDb::empty().unwrap();
    let f = fixture("F6");
    let data = &f.snapshots()["coverage"];
    for session in data["sessions"].as_array().unwrap() {
        seed(
            &mut db,
            session["session_id"].as_str().unwrap(),
            "cursor",
            session["surface"].as_str(),
            &serde_json::from_value::<Vec<CanonicalRecord>>(session["records"].clone()).unwrap(),
        );
    }
    let report = query(&db, &[]);
    assert_eq!(
        (
            report.usage.total.sessions,
            report.usage.total.measured,
            report.usage.total.pct
        ),
        (5, 1, Some(20.0))
    );
    assert_eq!(
        report.usage.total.gaps,
        vec![
            UsageGap::NoSelectedUsage,
            UsageGap::IncompleteCounters,
            UsageGap::UnknownModel
        ]
    );
    assert!(
        report
            .usage
            .by_surface
            .iter()
            .any(|s| s.surface.is_none() && s.usage.sessions == 1)
    );
    for (id, host) in [("F7", "cursor"), ("F10", "claude")] {
        let f = fixture(id);
        let data = &f.snapshots()["coverage"];
        seed(
            &mut db,
            id,
            host,
            data["surface"].as_str(),
            &serde_json::from_value::<Vec<CanonicalRecord>>(data["records"].clone()).unwrap(),
        );
    }
    let report = query(&db, &[]);
    assert_eq!(
        (report.usage.total.sessions, report.usage.total.measured),
        (7, 1)
    );
    assert_eq!(report.gate_14d.eligible_sessions, 6);
    assert_eq!(report.gate_14d.excluded_surfaces.len(), 1);
    assert_eq!(
        report.gate_14d.excluded_surfaces[0].surface.as_deref(),
        Some("cursor-cli")
    );
    assert_eq!(report.usage.by_host.len(), 2);
}

#[test]
fn coverage_fixed_gate_ninety_percent_and_empty_are_not_display_window() {
    let mut db = TempDb::empty().unwrap();
    assert_eq!(query(&db, &[]).gate_14d.passes, None);
    for i in 0..10 {
        let mut row = record(&format!("gate-{i}"), i < 9);
        row.timestamp = Some("2026-08-30T12:00:00Z".into());
        seed(&mut db, &format!("gate-{i}"), "claude", Some("cli"), &[row]);
    }
    let report = query(&db, &[]);
    assert_eq!(report.usage.total.sessions, 0);
    assert_eq!(
        (
            report.gate_14d.eligible_sessions,
            report.gate_14d.measured_sessions,
            report.gate_14d.pct,
            report.gate_14d.passes
        ),
        (10, 9, Some(90.0), Some(true))
    );
    let wide = coverage_in(
        &MetricsDb::open(db.path()).unwrap(),
        Window::new(ms("2026-08-01T00:00:00Z"), window().end_ms()).unwrap(),
        window().end_ms(),
        &[],
    )
    .unwrap();
    assert_eq!(wide.gate_14d, report.gate_14d);
    assert_eq!(wide.usage.total.sessions, 10);
    // A display window equal to the gate's own window gives the same gate.
    let metrics = MetricsDb::open(db.path()).unwrap();
    let same = coverage_in(&metrics, fourteen_days(), window().end_ms(), &[]).unwrap();
    assert_eq!(same.gate_14d, report.gate_14d);
    assert_eq!(same.usage.total.measured, 9);
    // One millisecond apart, the gate is its own window, not the display's.
    let shifted = Window::new(fourteen_days().start_ms() + 1, window().end_ms()).unwrap();
    let shifted = coverage_in(&metrics, shifted, window().end_ms(), &[]).unwrap();
    assert_eq!(shifted.gate_14d, report.gate_14d);
    seed(&mut db, "extra", "claude", None, &[record("extra", false)]);
    assert_eq!(query(&db, &[]).gate_14d.passes, Some(false));
    // Only the exact structural pair is excluded, never unknown/raw-new surfaces.
    let mut only = TempDb::empty().unwrap();
    seed(
        &mut only,
        "cli",
        "cursor",
        Some("cursor-cli"),
        &[record("cli", false)],
    );
    assert_eq!(query(&only, &[]).gate_14d.passes, None);
    let same = coverage_in(
        &MetricsDb::open(only.path()).unwrap(),
        fourteen_days(),
        window().end_ms(),
        &[],
    )
    .unwrap();
    assert_eq!(same.gate_14d, query(&only, &[]).gate_14d);
    assert_eq!(same.usage.total.sessions, 1);
    seed(
        &mut only,
        "other-host",
        "claude",
        Some("cursor-cli"),
        &[record("other-host", false)],
    );
    assert_eq!(query(&only, &[]).gate_14d.eligible_sessions, 1);
}

#[test]
fn coverage_selected_response_and_precise_membership_precede_measurement() {
    let mut db = TempDb::empty().unwrap();
    let mut old = record("old-partial", false);
    old.message.id = Some("shared".into());
    old.request_id = Some("req".into());
    old.message.usage = Some(serde_json::from_value(json!({"input_tokens":1})).unwrap());
    let mut final_row = record("final-complete", true);
    final_row.message.id = Some("shared".into());
    final_row.request_id = Some("req".into());
    final_row.timestamp = Some("2026-09-08T00:00:00Z".into());
    seed(
        &mut db,
        "boundary",
        "claude",
        Some("cli"),
        &[old, final_row],
    );
    assert_eq!(
        (
            query(&db, &[]).usage.total.sessions,
            query(&db, &[]).usage.total.measured
        ),
        (1, 0)
    );
    let mut leap = record("leap", true);
    leap.timestamp = Some("2026-09-07T23:59:60.9Z".into());
    seed(&mut db, "leap", "claude", Some("cli"), &[leap]);
    assert_eq!(query(&db, &[]).usage.total.measured, 1);
    let mut unknown = record("untimed", true);
    unknown.timestamp = None;
    seed(&mut db, "untimed", "claude", Some("cli"), &[unknown]);
    assert_eq!(query(&db, &[]).usage.total.sessions, 2);
}

#[test]
fn coverage_f20_native_start_capture_independence_and_discovery_only() {
    let f = fixture("F20");
    let data = &f.snapshots()["coverage"];
    let mut db = TempDb::empty().unwrap();
    let discoveries: Vec<DiscoveredSession> =
        serde_json::from_value(data["discovery"].clone()).unwrap();
    for d in discoveries
        .iter()
        .filter(|d| d.surface.is_some() && d.started_at_ms.is_some())
    {
        db.store_mut().observe_discovered_session(d).unwrap();
    }
    for (id, surface) in [
        ("coverage-cli", "claude-cli"),
        ("coverage-desktop", "claude-desktop"),
        ("coverage-old-start", "claude-cli"),
    ] {
        seed(&mut db, id, "claude", Some(surface), &[record(id, true)]);
    }
    receipt(
        &mut db,
        "cli-partial",
        "coverage-cli",
        Some("claude-cli"),
        "coverage-cli",
    );
    let contexts: Vec<_> = [
        "claude-cli",
        "claude-desktop",
        "future.app",
        "identity-missing",
        "incomplete.app",
        "empty",
    ]
    .into_iter()
    .map(|s| health("claude", Some(s), InventoryState::FreshComplete))
    .collect();
    let report = query(&db, &contexts);
    let capture = |surface: &str| {
        report
            .capture
            .iter()
            .find(|s| s.surface.as_deref() == Some(surface))
            .unwrap()
    };
    assert_eq!(
        (
            capture("claude-cli").observed_sessions,
            capture("claude-cli").captured_sessions,
            capture("claude-cli").pct
        ),
        (1, 1, Some(100.0))
    );
    assert_eq!(
        (
            capture("claude-desktop").observed_sessions,
            capture("claude-desktop").captured_sessions,
            capture("claude-desktop").pct
        ),
        (1, 0, Some(0.0))
    );
    assert_eq!(
        (
            capture("future.app").observed_sessions,
            capture("future.app").pct
        ),
        (1, Some(0.0))
    ); // No canonical session needed for discovery denominator.
    assert!(
        capture("identity-missing")
            .incomplete_reasons
            .contains(&CaptureGap::MissingConversationIdentity)
    );
    assert!(
        capture("incomplete.app")
            .incomplete_reasons
            .contains(&CaptureGap::DiscoveryIncomplete)
    );
    assert_eq!(capture("empty").pct, None);
    let missing_start = discoveries
        .iter()
        .find(|d| d.started_at_ms.is_none())
        .unwrap();
    db.store_mut()
        .observe_discovered_session(missing_start)
        .unwrap();
    let report = query(&db, &contexts);
    let cli = report
        .capture
        .iter()
        .find(|s| s.surface.as_deref() == Some("claude-cli"))
        .unwrap();
    assert_eq!(
        (
            cli.observed_sessions,
            cli.captured_sessions,
            cli.unknown_start_sessions,
            cli.pct
        ),
        (1, 1, 1, None)
    );
    assert!(
        cli.incomplete_reasons
            .contains(&CaptureGap::MissingNativeStart)
    );
}

#[test]
fn coverage_unknown_surface_denominator_uncertainty_is_host_and_window_scoped() {
    for start in [
        Some(ms("2026-09-07T12:00:00Z")),
        None,
        Some(ms("2026-08-01T00:00:00Z")),
    ] {
        let mut db = TempDb::empty().unwrap();
        for (id, host, surface) in [
            ("cli", "claude", "cli"),
            ("desktop", "claude", "desktop"),
            ("other", "cursor", "desktop"),
        ] {
            seed(&mut db, id, host, Some(surface), &[record(id, true)]);
            discover(
                &mut db,
                id,
                Some(id),
                Host::from_platform(host),
                Some(surface),
                Some(ms("2026-09-07T12:00:00Z")),
            );
            receipt(&mut db, id, id, Some(surface), id);
        }
        discover(
            &mut db,
            "unknown",
            Some("unknown"),
            Host::Claude,
            None,
            start,
        );
        let context = [
            health("claude", Some("cli"), InventoryState::FreshComplete),
            health("claude", Some("desktop"), InventoryState::FreshComplete),
            health("cursor", Some("desktop"), InventoryState::FreshComplete),
        ];
        for s in query(&db, &context).capture {
            if s.host == "cursor" {
                assert_eq!(s.pct, Some(100.0));
                continue;
            }
            let uncertain = start.is_none_or(|s| s >= window().start_ms());
            if uncertain {
                assert_eq!(s.pct, None);
                assert!(
                    s.incomplete_reasons
                        .contains(&CaptureGap::UnresolvedSurfaceDenominator)
                );
            } else {
                assert_eq!(s.pct, Some(100.0));
            }
            if s.surface.is_some() {
                assert_eq!(s.inventory, InventoryState::FreshComplete);
                assert_eq!(s.captured_sessions, 1);
                assert!(!s.incomplete_reasons.contains(&CaptureGap::MissingPython));
            }
        }
    }
}

#[test]
fn coverage_runtime_inventory_context_is_required_and_never_inferred() {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, "s", "claude", Some("cli"), &[record("s", true)]);
    discover(
        &mut db,
        "s",
        Some("s"),
        Host::Claude,
        Some("cli"),
        Some(ms("2026-09-07T12:00:00Z")),
    );
    receipt(&mut db, "r", "s", Some("cli"), "s");
    assert_eq!(query(&db, &[]).capture[0].pct, None);
    for (state, reason) in [
        (InventoryState::Unknown, CaptureGap::InventoryUnknown),
        (InventoryState::Incomplete, CaptureGap::InventoryIncomplete),
        (InventoryState::Stale, CaptureGap::InventoryStale),
        (InventoryState::MissingPython, CaptureGap::MissingPython),
        (InventoryState::Unavailable, CaptureGap::StoreUnavailable),
    ] {
        let report = query(&db, &[health("claude", Some("cli"), state)]);
        assert_eq!(report.capture[0].captured_sessions, 1);
        assert_eq!(report.capture[0].pct, None);
        assert!(report.capture[0].incomplete_reasons.contains(&reason));
    }
    let context = health("claude", Some("cli"), InventoryState::FreshComplete);
    assert_eq!(
        query(&db, std::slice::from_ref(&context)).capture[0].pct,
        Some(100.0)
    );
    assert!(
        coverage_in(
            &MetricsDb::open(db.path()).unwrap(),
            window(),
            window().end_ms(),
            &[context.clone(), context]
        )
        .is_err()
    );
}

#[test]
fn coverage_f18_usage_enrichment_and_partial_receipt_are_independent() {
    let f = fixture("F18");
    let data = &f.snapshots()["tokens"];
    for order in [["native", "hook"], ["hook", "native"]] {
        let mut db = TempDb::empty().unwrap();
        for source in order {
            seed(
                &mut db,
                "s",
                "claude",
                Some("cli"),
                &serde_json::from_value::<Vec<CanonicalRecord>>(data[source].clone()).unwrap(),
            );
        }
        let extra = record("not-receipted", true);
        seed(&mut db, "s", "claude", Some("cli"), &[extra]);
        discover(
            &mut db,
            "s",
            Some("s"),
            Host::Claude,
            Some("cli"),
            Some(ms("2026-09-07T12:00:00Z")),
        );
        let contexts = [health("claude", Some("cli"), InventoryState::FreshComplete)];
        let before = query(&db, &contexts);
        assert_eq!(before.usage.total.measured, 1);
        assert_eq!(before.capture[0].captured_sessions, 0);
        let uuid = data["native"][0]["uuid"].as_str().unwrap();
        receipt(&mut db, "partial", "s", Some("cli"), uuid);
        let after = query(&db, &contexts);
        assert_eq!(after.usage, before.usage);
        assert_eq!(after.capture[0].captured_sessions, 1);
        assert_eq!(db.store().capture_coverage("partial").unwrap().len(), 1);
        assert_eq!(db.store().records("s").unwrap().len(), 2);
        assert!(
            db.store()
                .records("s")
                .unwrap()
                .iter()
                .all(|r| r.content_json.is_none())
        );
    }
}

#[test]
fn coverage_capture_requires_sealed_known_matching_identity_and_surface() {
    for (label, stored_host, stored_surface, receipt_surface, reason) in [
        (
            "host",
            "cursor",
            Some("cli"),
            Some("cli"),
            Some(CaptureGap::IdentityMismatch),
        ),
        (
            "session-surface",
            "claude",
            Some("desktop"),
            Some("cli"),
            Some(CaptureGap::SurfaceMismatch),
        ),
        (
            "receipt-surface",
            "claude",
            Some("cli"),
            Some("desktop"),
            None,
        ),
        (
            "unknown",
            "claude",
            None,
            None,
            Some(CaptureGap::UnknownSurface),
        ),
    ] {
        let mut db = TempDb::empty().unwrap();
        seed(
            &mut db,
            label,
            stored_host,
            stored_surface,
            &[record(label, true)],
        );
        let discovered_surface = if label == "unknown" {
            None
        } else {
            Some("cli")
        };
        discover(
            &mut db,
            label,
            Some(label),
            Host::Claude,
            discovered_surface,
            Some(ms("2026-09-07T12:00:00Z")),
        );
        receipt(&mut db, "r", label, receipt_surface, label);
        let report = query(
            &db,
            &[health(
                "claude",
                discovered_surface,
                InventoryState::FreshComplete,
            )],
        );
        assert_eq!(report.capture[0].captured_sessions, 0);
        if let Some(reason) = reason {
            assert!(report.capture[0].incomplete_reasons.contains(&reason));
            assert_eq!(report.capture[0].pct, None);
        }
    }
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "unsealed",
        "claude",
        Some("cli"),
        &[record("unsealed", true)],
    );
    discover(
        &mut db,
        "unsealed",
        Some("unsealed"),
        Host::Claude,
        Some("cli"),
        Some(ms("2026-09-07T12:00:00Z")),
    );
    Connection::open(db.path()).unwrap().execute("INSERT INTO capture_receipts(receipt_id,session_id,surface,received_at) VALUES ('unsealed','unsealed','cli',0)",[]).unwrap();
    assert_eq!(
        query(
            &db,
            &[health("claude", Some("cli"), InventoryState::FreshComplete)]
        )
        .capture[0]
            .captured_sessions,
        0
    );
}

#[test]
fn coverage_open_checks_new_discovery_and_receipt_columns_without_repair() {
    for (table, column) in [
        ("discovered_sessions", "discovery_complete"),
        ("capture_receipts", "coverage_sealed"),
    ] {
        let db = TempDb::empty().unwrap();
        let c = Connection::open(db.path()).unwrap();
        c.execute_batch(&format!(
            "ALTER TABLE {table} RENAME COLUMN {column} TO obsolete"
        ))
        .unwrap();
        let before: String = c
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name=?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert!(MetricsDb::open(db.path()).is_err());
        let after: String = c
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name=?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, after);
        c.execute_batch(&format!(
            "ALTER TABLE {table} RENAME COLUMN obsolete TO {column}"
        ))
        .unwrap();
        assert!(MetricsDb::open(db.path()).is_ok());
    }
}
