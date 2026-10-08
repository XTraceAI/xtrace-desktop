use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{FavoriteModel, FavoriteUnknown, MetricsDb, PriceCatalog, Window};
use xt_store::{CanonicalRecord, retention::RetentionMode};
mod report_support;
fn fixture(id: &str) -> Fixture {
    Fixture::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(id),
    )
    .unwrap()
}
fn ms(ts: &str) -> i64 {
    ts.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn seed(db: &mut TempDb, id: &str, rows: &[CanonicalRecord], keep: bool) {
    let mut session = fixture("F1").sessions()[0].metadata.clone();
    session.session_id = id.into();
    db.store_mut().upsert_session(&session, keep).unwrap();
    db.store_mut().upsert_records(id, rows, keep).unwrap();
}
fn assistant(id: &str, second: u32, model: Option<&str>, output: Option<u64>) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":id,"type":"assistant","timestamp":format!("2026-09-07T12:00:{second:02}Z"),"message":{"role":"assistant","model":model,"content":[{"type":"text","text":"Synthetic"}],"usage":{"input_tokens":1,"output_tokens":output}}})).unwrap()
}
fn human(id: &str, second: u32) -> CanonicalRecord {
    let mut row = assistant(id, second, None, None);
    row.record_type = xt_store::model::RecordType::User;
    row.message.role = Some("user".into());
    row.message.usage = None;
    row
}
/// The favorite model, checked against the combined report read.
fn query(db: &TempDb) -> FavoriteModel {
    report_support::matches_standalone(
        &MetricsDb::open(db.path()).unwrap(),
        window(),
        window().end_ms(),
        &[],
        TimeZone::UTC,
        &PriceCatalog::bundled().unwrap(),
    )
    .current
    .favorite
    .unwrap()
}
#[test]
fn favorite_f11_actual_two_iso_weeks_turn_tie_and_categorical_comparison() {
    let f = fixture("F11");
    let snapshot = &f.snapshots()["favorite"];
    for keep in [false, true] {
        let mut db = TempDb::empty().unwrap();
        if keep {
            db.store_mut()
                .set_retention_mode(RetentionMode::FullContent)
                .unwrap();
        }
        for session in snapshot["sessions"].as_array().unwrap() {
            let rows: Vec<CanonicalRecord> =
                serde_json::from_value(session["records"].clone()).unwrap();
            seed(
                &mut db,
                session["session_id"].as_str().unwrap(),
                &rows,
                keep,
            );
            seed(
                &mut db,
                session["session_id"].as_str().unwrap(),
                &rows,
                keep,
            );
        }
        let metrics = MetricsDb::open(db.path()).unwrap();
        for week in snapshot["weeks"].as_array().unwrap() {
            let r = metrics
                .favorite_iso_week(ms(week["anchor"].as_str().unwrap()), TimeZone::UTC)
                .unwrap();
            assert_eq!(r.model.as_deref(), week["model"].as_str());
            assert_eq!(r.output_tokens, week["output_tokens"].as_u64());
            assert_eq!(r.unknown_reason, None);
        }
        let w = Window::iso_week(ms("2026-09-08T00:00:00Z"), TimeZone::UTC).unwrap();
        let compared = metrics.favorite_comparison(w).unwrap();
        assert_eq!(compared.current.model.as_deref(), Some("B"));
        assert_eq!(compared.previous.model.as_deref(), Some("A"));
        // The combined read decides both weeks, turn ties included, the same.
        let periods = report_support::matches_standalone(
            &metrics,
            w,
            w.end_ms(),
            &[],
            TimeZone::UTC,
            &PriceCatalog::bundled().unwrap(),
        );
        assert_eq!(periods.current.favorite.unwrap(), compared.current);
        assert_eq!(periods.previous.favorite.unwrap(), compared.previous);
    }
}
#[test]
fn favorite_unknown_output_bounds_and_partial_output_observations() {
    for (a, b, u, expected) in [
        (100, 80, 50, None),
        (140, 80, 50, Some("A")),
        (130, 80, 50, None),
        (100, 80, 0, Some("A")),
        (0, 0, 0, None),
    ] {
        let mut db = TempDb::empty().unwrap();
        seed(
            &mut db,
            "s",
            &[
                assistant("a", 0, Some("A"), Some(a)),
                assistant("b", 1, Some("B"), Some(b)),
                assistant("u", 2, None, Some(u)),
                assistant("partial", 3, Some("A"), None),
            ],
            false,
        );
        let r = query(&db);
        assert_eq!(r.model.as_deref(), expected);
        if expected.is_none() {
            assert_eq!(r.unknown_reason, Some(FavoriteUnknown::UnknownModelOutput));
        } else {
            assert_eq!(r.output_tokens, Some(a));
        }
    }
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[assistant("input-only", 0, Some("A"), None)],
        false,
    );
    assert_eq!(
        query(&db).unknown_reason,
        Some(FavoriteUnknown::NoMeasuredOutput)
    );
}
#[test]
fn favorite_fully_measured_turn_tie_is_lexical_and_first_assistant_owns_turn() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            human("h1", 0),
            assistant("b1", 1, Some("B"), Some(10)),
            assistant("a-later", 2, Some("A"), Some(10)),
            human("h2", 3),
            assistant("a-first", 4, Some("A"), Some(0)),
        ],
        false,
    );
    assert_eq!(query(&db).model.as_deref(), Some("A")); // one first-assistant turn each
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            human("h", 0),
            assistant("b1", 1, Some("B"), Some(10)),
            assistant("a-later", 2, Some("A"), Some(10)),
        ],
        false,
    );
    assert_eq!(query(&db).model.as_deref(), Some("B"));
}
#[test]
fn favorite_unknown_turn_facts_block_ties_but_not_provable_output_winners() {
    for field in ["is_human", "role", "model"] {
        let mut db = TempDb::empty().unwrap();
        seed(
            &mut db,
            "s",
            &[
                human("h", 0),
                assistant("first", 1, Some("B"), None),
                assistant("a", 2, Some("A"), Some(10)),
                assistant("b", 3, Some("B"), Some(10)),
            ],
            false,
        );
        let c = Connection::open(db.path()).unwrap();
        c.execute(
            &format!("UPDATE records SET {field}=NULL WHERE uuid='first'"),
            [],
        )
        .unwrap();
        assert_eq!(
            query(&db).unknown_reason,
            Some(FavoriteUnknown::UnknownTurnAttribution)
        );
        c.execute("UPDATE usage SET output_tokens=20 WHERE uuid='a'", [])
            .unwrap();
        assert_eq!(query(&db).model.as_deref(), Some("A"));
    }
    // A trailing unknown cannot change an already attributed turn.
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            human("h", 0),
            assistant("a", 1, Some("A"), Some(10)),
            assistant("b", 2, Some("B"), Some(10)),
            human("tail", 3),
        ],
        false,
    );
    Connection::open(db.path())
        .unwrap()
        .execute("UPDATE records SET is_human=NULL WHERE uuid='tail'", [])
        .unwrap();
    assert_eq!(query(&db).model.as_deref(), Some("A"));
}
#[test]
fn favorite_zero_unknown_output_does_not_itself_poison_positive_named_tie() {
    let mut zero_db = TempDb::empty().unwrap();
    seed(
        &mut zero_db,
        "zero",
        &[
            assistant("zero-b", 0, Some("B"), Some(0)),
            assistant("zero-a", 1, Some("A"), Some(0)),
        ],
        false,
    );
    assert_eq!(query(&zero_db).model.as_deref(), Some("A"));
    assert_eq!(query(&zero_db).output_tokens, Some(0));

    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            assistant("unknown-leading", 0, None, Some(0)),
            human("h", 1),
            assistant("a", 2, Some("A"), Some(10)),
            assistant("b", 3, Some("B"), Some(10)),
        ],
        false,
    );
    assert_eq!(query(&db).model.as_deref(), Some("A"));
}
#[test]
fn favorite_precise_window_dedupe_and_turn_order_share_canonical_views() {
    let mut db = TempDb::empty().unwrap();
    let mut h = human("h", 0);
    h.timestamp = Some("2026-09-07T12:00:00.00000000001Z".into());
    let mut b = assistant("z-first", 1, Some("B"), Some(10));
    b.timestamp = Some("2026-09-07T12:00:00.00000000002Z".into());
    let mut a = assistant("a-later", 2, Some("A"), Some(10));
    a.timestamp = Some("2026-09-07T05:00:00.00000000003-07:00".into());
    seed(&mut db, "s", &[a, b, h], false);
    assert_eq!(query(&db).model.as_deref(), Some("B"));
    // The later snapshot outside the window replaces the earlier in-window usage.
    let mut before = assistant("snap-in", 3, Some("C"), Some(1000));
    before.api_message_id = Some("response".into());
    before.request_id = Some("request".into());
    let mut after = before.clone();
    after.uuid = Some("snap-out".into());
    after.timestamp = Some("2026-09-08T00:00:00Z".into());
    seed(&mut db, "s", &[before, after], false);
    assert_eq!(query(&db).model.as_deref(), Some("B"));
}

#[test]
fn favorite_open_rejects_missing_turn_model_without_reader_repair() {
    let db = TempDb::empty().unwrap();
    let c = Connection::open(db.path()).unwrap();
    c.execute_batch("DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT uuid,session_id,host,surface,ts,ts_ms,role,is_human,text_len,tool_use_count,type FROM v_records WHERE ts_ms IS NOT NULL").unwrap();
    let before: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='v_session_events'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(MetricsDb::open(db.path()).is_err());
    let after: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='v_session_events'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
    xt_store::Store::open(db.path()).unwrap();
    assert!(MetricsDb::open(db.path()).is_ok());
}

#[test]
fn favorite_f11_lexical_tie_variant_retains_all_measured_output() {
    let f = fixture("F11");
    let mut db = TempDb::empty().unwrap();
    let session = &f.snapshots()["favorite"]["sessions"][0];
    let rows: Vec<CanonicalRecord> = serde_json::from_value(session["records"].clone()).unwrap();
    // Remove only the second A prompt: both A response blocks now belong to one
    // responding segment, leaving one A turn and one B turn at 100 outputs each.
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|row| row.uuid.as_deref() != Some("week-one-h1"))
        .collect();
    seed(&mut db, "lexical-variant", &rows, false);
    let r = MetricsDb::open(db.path())
        .unwrap()
        .favorite_iso_week(ms("2026-09-01T00:00:00Z"), TimeZone::UTC)
        .unwrap();
    assert_eq!(r.model.as_deref(), Some("A"));
    assert_eq!(r.output_tokens, Some(100));
    let week = Window::iso_week(ms("2026-09-01T00:00:00Z"), TimeZone::UTC).unwrap();
    let periods = report_support::matches_standalone(
        &MetricsDb::open(db.path()).unwrap(),
        week,
        week.end_ms(),
        &[],
        TimeZone::UTC,
        &PriceCatalog::bundled().unwrap(),
    );
    assert_eq!(periods.current.favorite.unwrap(), r);
}

#[test]
fn favorite_blank_model_output_is_unidentified_and_nonblank_ids_are_preserved() {
    for model in ["", " \t\u{2003}\u{3000}"] {
        let mut db = TempDb::empty().unwrap();
        seed(
            &mut db,
            "s",
            &[
                assistant("a", 0, Some("A"), Some(100)),
                assistant("b", 1, Some("B"), Some(80)),
                assistant("blank", 2, Some(model), Some(50)),
            ],
            false,
        );
        assert_eq!(
            query(&db).unknown_reason,
            Some(FavoriteUnknown::UnknownModelOutput)
        );
    }
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[assistant(
            "nonblank",
            0,
            Some(" model-A \u{2003}"),
            Some(100),
        )],
        false,
    );
    assert_eq!(query(&db).model.as_deref(), Some(" model-A \u{2003}"));
}

#[test]
fn favorite_blank_first_assistant_model_leaves_tied_attribution_unknown() {
    for model in ["", "\u{2003}\u{3000}\n"] {
        let mut db = TempDb::empty().unwrap();
        seed(
            &mut db,
            "s",
            &[
                human("h", 0),
                assistant("first", 1, Some(model), None),
                assistant("a", 2, Some("A"), Some(10)),
                assistant("b", 3, Some("B"), Some(10)),
            ],
            false,
        );
        assert_eq!(
            query(&db).unknown_reason,
            Some(FavoriteUnknown::UnknownTurnAttribution)
        );
    }
}
