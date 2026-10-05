use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{COST_BASIS, CostReport, MetricsDb, PriceCatalog, UnpricedReason, Window};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource};
fn fixture(id: &str) -> Fixture {
    Fixture::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(id),
    )
    .unwrap()
}
fn ms(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn catalog_json() -> Value {
    fixture("F1").snapshots()["prices"].clone()
}
fn catalog() -> PriceCatalog {
    PriceCatalog::from_json(&catalog_json().to_string()).unwrap()
}
fn query(db: &TempDb) -> CostReport {
    MetricsDb::open(db.path())
        .unwrap()
        .cost(window(), TimeZone::UTC, &catalog())
        .unwrap()
}
fn seed(db: &mut TempDb, id: &str, host: &str, surface: Option<&str>, rows: &[CanonicalRecord]) {
    let mut session = SessionMeta::new(id, host, SessionSource::Fixture);
    session.surface = surface.map(str::to_owned);
    db.store_mut().upsert_session(&session, false).unwrap();
    db.store_mut().upsert_records(id, rows, false).unwrap();
}
fn record(
    id: &str,
    model: &str,
    tier: &str,
    counters: [i64; 4],
    split: Option<[i64; 2]>,
) -> CanonicalRecord {
    let [input, output, read, write] = counters;
    let mut v = json!({"uuid":id,"type":"assistant","timestamp":"2026-09-07T12:00:00Z","message":{"role":"assistant","model":model,"content":[],"usage":{"input_tokens":input,"output_tokens":output,"cache_read_input_tokens":read,"cache_creation_input_tokens":write,"service_tier":tier}}});
    if let Some([five, hour]) = split {
        v["message"]["usage"]["cache_creation"] =
            json!({"ephemeral_5m_input_tokens":five,"ephemeral_1h_input_tokens":hour});
    }
    serde_json::from_value(v).unwrap()
}
#[test]
fn cost_synthetic_four_counter_tier_alias_and_reasoning_once() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "claude",
        "claude",
        Some("cli"),
        &[record(
            "claude",
            "test-claude",
            "standard",
            [1000, 100, 2000, 500],
            Some([300, 200]),
        )],
    );
    // Canonical output already includes reasoning. Extra informational detail is
    // ignored by the canonical model and is never another billable output counter.
    let mut codex = serde_json::to_value(record(
        "codex",
        "test-flat",
        "standard",
        [1000, 300, 2000, 500],
        None,
    ))
    .unwrap();
    codex["message"]["usage"]["output_tokens_details"] = json!({"reasoning_tokens":100});
    seed(
        &mut db,
        "codex",
        "codex",
        None,
        &[serde_json::from_value(codex).unwrap()],
    );
    seed(
        &mut db,
        "cursor",
        "cursor",
        Some("desktop"),
        &[record(
            "cursor",
            "test-flat-alias",
            "standard",
            [10, 20, 30, 40],
            None,
        )],
    );
    let report = query(&db);
    assert_eq!(report.price_version, "synthetic-v1");
    assert_eq!(report.basis, COST_BASIS);
    assert_eq!(report.total.total_usd, Some(0.008063));
    assert_eq!(report.total.priced_observations, 3);
    for (host, expected) in [
        ("claude", 0.002975),
        ("codex", 0.004825),
        ("cursor", 0.000263),
    ] {
        assert_eq!(
            report
                .by_host
                .iter()
                .find(|r| r.host == host)
                .unwrap()
                .cost
                .total_usd,
            Some(expected)
        );
    }
    assert_eq!(report.by_model.len(), 3);
    assert_eq!(report.by_surface.len(), 3);
    assert_eq!(
        report
            .by_day
            .iter()
            .find(|d| d.date == "2026-09-07")
            .unwrap()
            .cost,
        report.total
    );
    seed(
        &mut db,
        "fast",
        "claude",
        None,
        &[record(
            "fast",
            "test-claude",
            "test-fast",
            [1000, 100, 2000, 500],
            Some([300, 200]),
        )],
    );
    assert_eq!(
        query(&db)
            .by_model
            .iter()
            .find(|r| r.model.as_deref() == Some("test-claude"))
            .unwrap()
            .cost
            .total_usd,
        Some(0.008925)
    );
}
#[test]
fn cost_unknowns_keep_named_model_tier_and_known_subtotal() {
    let mut db = TempDb::empty().unwrap();
    let mut rows = vec![
        record("known", "test-flat", "standard", [100, 0, 0, 0], None),
        record("unknown-model", "unknown-model", "standard", [0; 4], None),
        record("tier", "test-flat", "mystery-tier", [0; 4], None),
        record("split", "test-claude", "standard", [1, 1, 1, 5], None),
        record(
            "bad-split",
            "test-claude",
            "standard",
            [1, 1, 1, 5],
            Some([1, 1]),
        ),
    ];
    let mut missing = record("missing-tier", "test-flat", "standard", [0; 4], None);
    missing.message.usage.as_mut().unwrap().service_tier = None;
    rows.push(missing);
    let mut counters = record("missing-counter", "test-flat", "standard", [0; 4], None);
    counters.message.usage.as_mut().unwrap().output_tokens = None;
    rows.push(counters);
    seed(&mut db, "s", "claude", None, &rows);
    let r = query(&db);
    assert_eq!(r.total.total_usd, None);
    assert_eq!(r.total.priced_subtotal_usd, 0.0001);
    assert_eq!(r.total.unpriced_observations, 6);
    assert!(
        r.total
            .unpriced
            .iter()
            .any(|u| u.model.as_deref() == Some("unknown-model")
                && u.reason == UnpricedReason::UnknownModel)
    );
    assert!(
        r.total
            .unpriced
            .iter()
            .any(|u| u.service_tier.as_deref() == Some("mystery-tier")
                && u.reason == UnpricedReason::UnknownServiceTier)
    );
    for reason in [
        UnpricedReason::MissingServiceTier,
        UnpricedReason::MissingCounters,
        UnpricedReason::MissingCacheSplit,
        UnpricedReason::InconsistentCacheSplit,
    ] {
        assert!(r.total.unpriced.iter().any(|u| u.reason == reason));
    }
    let tokens = MetricsDb::open(db.path())
        .unwrap()
        .tokens(window(), TimeZone::UTC)
        .unwrap();
    assert_eq!(tokens.total.selected_responses, 7);
}
#[test]
fn cost_zero_writes_allow_absent_ttl_but_reject_contradictory_split() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        "claude",
        None,
        &[record("zero", "test-claude", "standard", [0; 4], None)],
    );
    assert_eq!(query(&db).total.total_usd, Some(0.0));
    seed(
        &mut db,
        "s",
        "claude",
        None,
        &[record(
            "bad",
            "test-claude",
            "standard",
            [0; 4],
            Some([1, 0]),
        )],
    );
    assert_eq!(query(&db).total.total_usd, None);
    assert_eq!(
        query(&db).total.unpriced[0].reason,
        UnpricedReason::InconsistentCacheSplit
    );
    let empty = query(&TempDb::empty().unwrap());
    assert_eq!(empty.total.total_usd, None);
    assert_eq!(empty.total.selected_observations, 0);
}
#[test]
fn cost_bundled_exact_ids_tiers_and_long_context_boundary() {
    let prices = PriceCatalog::bundled().unwrap();
    assert_eq!(prices.as_of(), "2026-10-01");
    for (model, tier, rate) in [
        ("gpt-6-sol", "default", 0.000002),
        ("gpt-6.1-sol", "default", 0.000002),
        ("gpt-5.5", "default", 0.000005),
        ("claude-opus-5-5", "standard", 0.000004),
        ("claude-fable-5-1", "standard", 0.00001),
        ("claude-fable-5", "standard", 0.00001),
        ("gpt-6-astra", "default", 0.000010),
        ("gpt-5.6-sol", "default", 0.000004),
        ("gpt-5.6", "default", 0.000004),
        ("gpt-5.6-terra", "default", 0.000002),
        ("gpt-5.6-luna", "default", 0.0000002),
        ("claude-opus-5", "standard", 0.000005),
        ("claude-sonnet-5", "standard", 0.000002),
        ("claude-haiku-4-5-20251001", "standard", 0.000001),
    ] {
        let mut db = TempDb::empty().unwrap();
        seed(
            &mut db,
            "s",
            "cursor",
            None,
            &[record("s", model, tier, [1, 0, 0, 0], None)],
        );
        assert_eq!(
            MetricsDb::open(db.path())
                .unwrap()
                .cost(window(), TimeZone::UTC, &prices)
                .unwrap()
                .total
                .total_usd,
            Some(rate)
        );
    }
    for (counters, expected) in [
        ([272000, 2, 0, 0], 2.7201),
        ([272000, 2, 1, 0], 5.440152),
        ([272000, 2, 0, 1], 5.440175),
    ] {
        let mut db = TempDb::empty().unwrap();
        seed(
            &mut db,
            "s",
            "codex",
            None,
            &[record("s", "gpt-6-astra", "default", counters, None)],
        );
        let metrics = MetricsDb::open(db.path()).unwrap();
        assert_eq!(
            metrics
                .cost(window(), TimeZone::UTC, &prices)
                .unwrap()
                .total
                .total_usd,
            Some(expected)
        );
    }
    for (tier, factor) in [
        ("default", 1.0),
        ("flex", 0.5),
        ("fast", 2.0),
        ("priority", 2.0),
    ] {
        let mut db = TempDb::empty().unwrap();
        seed(
            &mut db,
            "s",
            "codex",
            None,
            &[record("s", "gpt-6-astra", tier, [1, 0, 0, 0], None)],
        );
        assert_eq!(
            MetricsDb::open(db.path())
                .unwrap()
                .cost(window(), TimeZone::UTC, &prices)
                .unwrap()
                .total
                .total_usd,
            Some(0.00001 * factor)
        );
    }
    for (model, tier) in [
        ("gpt-6-astra", "standard"),
        ("gpt-6-astra", "auto"),
        ("gpt-6-astra", "batch"),
        ("claude-opus-5", "priority"),
        ("claude-haiku-4-5", "standard"),
        ("gpt-6-astra-20260101", "default"),
    ] {
        let mut db = TempDb::empty().unwrap();
        seed(
            &mut db,
            "s",
            "codex",
            None,
            &[record("s", model, tier, [0; 4], None)],
        );
        assert_eq!(
            MetricsDb::open(db.path())
                .unwrap()
                .cost(window(), TimeZone::UTC, &prices)
                .unwrap()
                .total
                .priced_observations,
            0
        );
    }
}
#[test]
fn cost_catalog_version_changes_history_and_invalid_catalogs_fail_closed() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        "codex",
        None,
        &[record("s", "test-flat", "standard", [1000, 0, 0, 0], None)],
    );
    let old = catalog();
    let mut new = catalog_json();
    new["version"] = json!("synthetic-v2");
    new["models"][0]["bands"][0]["tiers"][0]["rates"]["input"] = json!(2000);
    let new = PriceCatalog::from_json(&new.to_string()).unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    assert_eq!(
        metrics
            .cost(window(), TimeZone::UTC, &old)
            .unwrap()
            .total
            .total_usd,
        Some(0.001)
    );
    let result = metrics.cost(window(), TimeZone::UTC, &new).unwrap();
    assert_eq!(result.price_version, "synthetic-v2");
    assert_eq!(result.total.total_usd, Some(0.002));
    for bad in [json!(null), json!({"version":"bad"})] {
        assert!(PriceCatalog::from_json(&bad.to_string()).is_err());
    }
    for (pointer, value) in [
        ("/as_of", json!("2026-02-30")),
        ("/version", json!(" ")),
        ("/basis", json!("invoice")),
        ("/sources", json!([])),
        ("/models/0/aliases", json!(["fixture-model"])),
        ("/models/0/bands/0/max_prompt_tokens", json!(100)),
        ("/models/0/bands/0/tiers/0/rates/input", json!(-1)),
        ("/models/0/bands/0/tiers/0/rates/input", json!(0.1)),
        (
            "/models/0/bands/0/tiers/0/names",
            json!(["standard", "standard"]),
        ),
        ("/models/0/bands/0/tiers/0/rates/cache_write_5m", json!(1)),
    ] {
        let mut bad = catalog_json();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(
            PriceCatalog::from_json(&bad.to_string()).is_err(),
            "{pointer}"
        );
    }
    let mut bad = catalog_json();
    bad["extra"] = json!(true);
    assert!(PriceCatalog::from_json(&bad.to_string()).is_err());
    assert_eq!(
        metrics
            .cost(window(), TimeZone::UTC, &old)
            .unwrap()
            .total
            .total_usd,
        Some(0.001)
    );
}

fn priced_rows(value: &Value) -> Vec<CanonicalRecord> {
    let mut rows: Vec<CanonicalRecord> = serde_json::from_value(value.clone()).unwrap();
    // These named token fixtures omit tier. Supply explicit synthetic tier evidence
    // to their canonical inputs; do not imply that real missing tiers are priced.
    for row in &mut rows {
        if let Some(usage) = &mut row.message.usage {
            usage.service_tier = Some("standard".into());
        }
    }
    rows
}
#[test]
fn cost_f1_f5_f6_and_f18_enrichment_use_same_version_without_storage_cache() {
    let f = fixture("F1");
    let mut db = f.build_db(false).unwrap();
    assert_eq!(query(&db).total.priced_observations, 0);
    let mut rows = f.sessions()[0].records.clone();
    for row in &mut rows {
        if let Some(usage) = &mut row.message.usage {
            usage.service_tier = Some("standard".into());
        }
    }
    let stats = db
        .store_mut()
        .upsert_records(&f.sessions()[0].metadata.session_id, &rows, false)
        .unwrap();
    assert_eq!(stats.inserted, 0);
    assert!(stats.enriched > 0);
    assert_eq!(query(&db).total.total_usd, Some(0.0023275));
    for (id, host, expected) in [("F5", "codex", 0.000473), ("F6", "cursor", 0.00012)] {
        let f = fixture(id);
        let rows = priced_rows(&f.snapshots()["tokens"]["records"]);
        let mut db = TempDb::empty().unwrap();
        seed(&mut db, id, host, None, &rows);
        assert_eq!(query(&db).total.total_usd, Some(expected));
        assert_eq!(query(&db).total.selected_observations, 1);
    }
    let f = fixture("F18");
    let data = &f.snapshots()["tokens"];
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, "s", "claude", None, &priced_rows(&data["native"]));
    assert_eq!(query(&db).total.selected_observations, 0);
    assert_eq!(query(&db).total.total_usd, None);
    let rows = priced_rows(&data["hook"]);
    let stats = db.store_mut().upsert_records("s", &rows, false).unwrap();
    assert_eq!((stats.inserted, stats.enriched), (0, 1));
    let after = query(&db);
    assert_eq!(after.price_version, "synthetic-v1");
    assert_eq!(after.total.total_usd, Some(0.00012));
    db.store_mut().upsert_records("s", &rows, false).unwrap();
    assert_eq!(query(&db), after);
    assert!(
        db.store()
            .records("s")
            .unwrap()
            .iter()
            .all(|r| r.content_json.is_none())
    );
}
#[test]
fn cost_f17_representative_replacement_and_window_day_membership() {
    let f = fixture("F17");
    let rows = priced_rows(&f.snapshots()["tokens"]["records"]);
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, "s", "claude", None, &rows[..1]);
    assert_eq!(query(&db).total.total_usd, Some(0.00002));
    seed(&mut db, "s", "claude", None, &rows[1..2]);
    assert_eq!(query(&db).total.total_usd, Some(0.00004));
    assert_eq!(query(&db).total.selected_observations, 1);
    assert_eq!(
        query(&db)
            .by_day
            .iter()
            .find(|d| d.date == "2026-09-01")
            .unwrap()
            .cost
            .selected_observations,
        0
    );
    seed(&mut db, "s", "claude", None, &rows[2..]);
    let report = query(&db);
    let token = query_tokens(&db);
    let expected = (token.total.counters.input_tokens.unwrap() * 1000
        + token.total.counters.output_tokens.unwrap() * 10000
        + token.total.counters.cache_read_tokens.unwrap() * 100
        + token.total.counters.cache_creation_tokens.unwrap() * 1250) as f64
        / 1e9;
    assert_eq!(expected, 0.000088);
    // F17 deliberately contains a selected observation with unknown model.
    assert_eq!(report.total.total_usd, None);
    assert_eq!(report.total.priced_subtotal_usd, 0.000073);
    assert_eq!(
        report.total.unpriced[0].reason,
        UnpricedReason::MissingModel
    );
    assert_eq!(
        report.total.selected_observations,
        token.total.selected_responses
    );
    let next = Window::new(window().end_ms(), window().end_ms() + 86_400_000).unwrap();
    assert_eq!(
        MetricsDb::open(db.path())
            .unwrap()
            .cost(next, TimeZone::UTC, &catalog())
            .unwrap()
            .total
            .total_usd,
        Some(0.00008)
    );
    let c = Connection::open(db.path()).unwrap();
    c.execute("UPDATE sessions SET kind='judge'", []).unwrap();
    assert_eq!(query(&db).total.selected_observations, 0);
}
fn query_tokens(db: &TempDb) -> xt_metrics::TokenReport {
    MetricsDb::open(db.path())
        .unwrap()
        .tokens(window(), TimeZone::UTC)
        .unwrap()
}
#[test]
fn cost_missing_rate_prompt_facts_batch_ttl_and_checked_overflow() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        "codex",
        None,
        &[record("s", "test-flat", "standard", [0; 4], None)],
    );
    let mut value = catalog_json();
    value["models"][0]["bands"][0]["tiers"][0]["rates"]["input"] = Value::Null;
    let partial = PriceCatalog::from_json(&value.to_string()).unwrap();
    assert_eq!(
        MetricsDb::open(db.path())
            .unwrap()
            .cost(window(), TimeZone::UTC, &partial)
            .unwrap()
            .total
            .unpriced[0]
            .reason,
        UnpricedReason::MissingRate
    );
    let mut db = TempDb::empty().unwrap();
    let mut row = record("s", "gpt-6-astra", "default", [0; 4], None);
    row.message.usage.as_mut().unwrap().cache_read_input_tokens = None;
    seed(&mut db, "s", "codex", None, &[row]);
    assert_eq!(
        MetricsDb::open(db.path())
            .unwrap()
            .cost(window(), TimeZone::UTC, &PriceCatalog::bundled().unwrap())
            .unwrap()
            .total
            .unpriced[0]
            .reason,
        UnpricedReason::MissingPromptCounters
    );
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        "claude",
        None,
        &[record(
            "s",
            "claude-haiku-4-5-20251001",
            "batch",
            [1000, 100, 2000, 500],
            Some([300, 200]),
        )],
    );
    assert_eq!(
        MetricsDb::open(db.path())
            .unwrap()
            .cost(window(), TimeZone::UTC, &PriceCatalog::bundled().unwrap())
            .unwrap()
            .total
            .total_usd,
        Some(0.0012375)
    );
    let mut value = catalog_json();
    for key in ["input", "output", "cache_read", "cache_write"] {
        value["models"][0]["bands"][0]["tiers"][0]["rates"][key] = json!(u64::MAX);
    }
    let overflow = PriceCatalog::from_json(&value.to_string()).unwrap();
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        "codex",
        None,
        &[record("s", "test-flat", "standard", [i64::MAX; 4], None)],
    );
    assert!(matches!(
        MetricsDb::open(db.path())
            .unwrap()
            .cost(window(), TimeZone::UTC, &overflow),
        Err(xt_metrics::Error::CounterOverflow)
    ));
}
#[test]
fn cost_open_checks_cache_split_and_tier_projection_without_repair() {
    for missing in ["cache_creation_5m", "cache_creation_1h", "service_tier"] {
        let db = TempDb::empty().unwrap();
        let c = Connection::open(db.path()).unwrap();
        let columns = [
            "session_id",
            "host",
            "model",
            "surface",
            "ts",
            "ts_ms",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_creation_tokens",
            "cache_creation_5m",
            "cache_creation_1h",
            "service_tier",
        ];
        let select = columns
            .into_iter()
            .filter(|column| *column != missing)
            .collect::<Vec<_>>()
            .join(",");
        c.execute_batch(&format!("DROP VIEW v_response_usage; CREATE VIEW v_response_usage AS SELECT {select} FROM v_usage_records")).unwrap();
        let before: String = c
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='v_response_usage'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(MetricsDb::open(db.path()).is_err());
        let after: String = c
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='v_response_usage'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, after);
        let _writer = xt_store::Store::open(db.path()).unwrap();
        assert!(MetricsDb::open(db.path()).is_ok());
    }
}
#[test]
fn cost_precise_and_leap_membership_use_same_day_as_tokens() {
    let mut db = TempDb::empty().unwrap();
    let mut rows = vec![
        record("in", "test-flat", "standard", [1, 0, 0, 0], None),
        record("out", "test-flat", "standard", [100, 0, 0, 0], None),
    ];
    rows[0].timestamp = Some("2026-09-07T23:59:60.9Z".into());
    rows[1].timestamp = Some("2026-09-08T00:00:00.0000000001Z".into());
    seed(&mut db, "s", "claude", None, &rows);
    let r = query(&db);
    assert_eq!(r.total.total_usd, Some(0.000001));
    assert_eq!(r.by_day.last().unwrap().cost, r.total);
    let next = Window::new(window().end_ms(), window().end_ms() + 1000).unwrap();
    assert_eq!(
        MetricsDb::open(db.path())
            .unwrap()
            .cost(next, TimeZone::UTC, &catalog())
            .unwrap()
            .total
            .total_usd,
        Some(0.0001)
    );
}

fn bundled(db: &TempDb) -> CostReport {
    MetricsDb::open(db.path())
        .unwrap()
        .cost(window(), TimeZone::UTC, &PriceCatalog::bundled().unwrap())
        .unwrap()
}
fn untiered(id: &str, model: &str, counters: [i64; 4]) -> CanonicalRecord {
    let mut row = record(id, model, "unused", counters, None);
    row.message.usage.as_mut().unwrap().service_tier = None;
    row
}
#[test]
fn cost_bundled_claude_ttl_split_rates_for_new_models() {
    for (model, tier, expected) in [
        ("claude-opus-5-5", "standard", 0.0095),
        ("claude-opus-5-5", "batch", 0.00475),
        ("claude-fable-5-1", "standard", 0.02325),
        ("claude-fable-5-1", "batch", 0.011625),
        ("claude-fable-5", "standard", 0.02475),
    ] {
        let mut db = TempDb::empty().unwrap();
        // 1,000 fresh, 100 output, 2,000 reads, 300 five-minute and 200
        // one-hour writes, each at its own rate.
        seed(
            &mut db,
            "s",
            "claude",
            None,
            &[record(
                "s",
                model,
                tier,
                [1000, 100, 2000, 500],
                Some([300, 200]),
            )],
        );
        let total = bundled(&db).total;
        assert_eq!(total.total_usd, Some(expected), "{model} {tier}");
        assert_eq!(total.assumed_tier_observations, 0);
    }
}
#[test]
fn cost_codex_missing_tier_is_priced_at_default_and_counted_as_assumed() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "codex",
        "codex",
        None,
        &[
            untiered("sol", "gpt-6-sol", [1000, 100, 2000, 500]),
            untiered("review", "codex-auto-review", [1000, 100, 0, 0]),
        ],
    );
    // Only Codex is assumed to bill at its default tier; another host's
    // missing tier stays unpriced.
    seed(
        &mut db,
        "claude",
        "claude",
        None,
        &[untiered("claude", "claude-opus-5-5", [1, 0, 0, 0])],
    );
    seed(
        &mut db,
        "cursor",
        "cursor",
        None,
        &[untiered("cursor", "gpt-6-sol", [1, 0, 0, 0])],
    );
    let report = bundled(&db);
    let total = &report.total;
    assert_eq!(
        (
            total.selected_observations,
            total.priced_observations,
            total.unpriced_observations,
            total.assumed_tier_observations
        ),
        (4, 1, 3, 1)
    );
    assert_eq!(total.total_usd, None);
    assert_eq!(total.priced_subtotal_usd, 0.00465);
    let reasons: Vec<_> = total
        .unpriced
        .iter()
        .map(|u| (u.model.as_deref(), u.reason, u.observations))
        .collect();
    assert_eq!(
        reasons,
        [
            (
                Some("claude-opus-5-5"),
                UnpricedReason::MissingServiceTier,
                1
            ),
            (Some("codex-auto-review"), UnpricedReason::UnknownModel, 1),
            (Some("gpt-6-sol"), UnpricedReason::MissingServiceTier, 1),
        ]
    );
    let codex = report.by_host.iter().find(|h| h.host == "codex").unwrap();
    assert_eq!(codex.cost.assumed_tier_observations, 1);
    // A recorded tier is used as recorded and is not an assumption.
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        "codex",
        None,
        &[record(
            "s",
            "gpt-6-sol",
            "flex",
            [1000, 100, 2000, 500],
            None,
        )],
    );
    let total = bundled(&db).total;
    assert_eq!(total.total_usd, Some(0.002325));
    assert_eq!(total.assumed_tier_observations, 0);
}
#[test]
fn cost_bundled_openai_long_context_bands_and_unpublished_rates() {
    for (model, tier, counters, expected) in [
        ("gpt-6-sol", None, [272000, 2, 0, 0], Some(0.54402)),
        ("gpt-6-sol", None, [272001, 2, 0, 0], Some(1.088034)),
        ("gpt-6-sol", Some("flex"), [272001, 2, 0, 0], Some(0.544017)),
        ("gpt-6-sol", Some("fast"), [272000, 0, 0, 1], Some(2.17601)),
        ("gpt-6.1-sol", None, [0, 0, 1000, 0], Some(0.0001)),
        ("gpt-6.1-sol", None, [272000, 0, 1, 0], Some(1.0880002)),
        ("gpt-5.5", Some("priority"), [1, 0, 0, 0], Some(0.0000125)),
        ("gpt-5.5", None, [272001, 0, 0, 0], Some(2.72001)),
        // Long-context priority rates are not published: unpriced, not zero.
        ("gpt-5.5", Some("priority"), [272001, 0, 0, 0], None),
    ] {
        let mut db = TempDb::empty().unwrap();
        let row = match tier {
            Some(tier) => record("s", model, tier, counters, None),
            None => untiered("s", model, counters),
        };
        seed(&mut db, "s", "codex", None, &[row]);
        let total = bundled(&db).total;
        assert_eq!(total.total_usd, expected, "{model} {tier:?} {counters:?}");
        if expected.is_none() {
            assert_eq!(total.unpriced[0].reason, UnpricedReason::MissingRate);
        }
    }
}
