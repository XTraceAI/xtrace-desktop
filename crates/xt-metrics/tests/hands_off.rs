mod hands_off_support;

use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{HandsOff, MetricsDb, Window};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, retention::RetentionMode};

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
/// The window's report, checked equal to the combined read's.
fn query(db: &TempDb) -> HandsOff {
    query_in(db, window())
}
fn query_in(db: &TempDb, window: Window) -> HandsOff {
    let metrics = MetricsDb::open(db.path()).unwrap();
    hands_off_support::matches_standalone(&metrics, window, TimeZone::UTC).0
}
fn seed(
    db: &mut TempDb,
    id: &str,
    host: &str,
    surface: Option<&str>,
    rows: &[CanonicalRecord],
    content: bool,
) {
    let mut session = SessionMeta::new(id, host, SessionSource::Fixture);
    session.surface = surface.map(str::to_owned);
    db.store_mut().upsert_session(&session, content).unwrap();
    db.store_mut().upsert_records(id, rows, content).unwrap();
}
fn record(id: &str, ts: &str, human: bool, tools: bool) -> CanonicalRecord {
    let role = if human { "user" } else { "assistant" };
    serde_json::from_value(json!({"uuid":id,"type":role,"timestamp":ts,"message":{"role":role,"content":if tools {json!([{"type":"tool_use","name":"Read","input":{"path":"synthetic"}}])} else {json!([{"type":"text","text":"Synthetic"}])}}})).unwrap()
}
fn sample() -> Vec<CanonicalRecord> {
    vec![
        record("h", "2026-09-07T12:00:00Z", true, false),
        record("a", "2026-09-07T12:01:00Z", false, true),
    ]
}

#[test]
fn hands_off_f1_exact_endpoints_and_metadata_replay_parity() {
    let f = fixture("F1");
    let golden = &f.snapshots()["hands_off"];
    for keep in [false, true] {
        let mut db = f.build_db(keep).unwrap();
        let expected = &golden["expected"];
        assert_eq!(serde_json::to_value(query(&db)).unwrap(), *expected);
        let input = &f.sessions()[0];
        let pairs: Vec<_> = input
            .records
            .chunks(5)
            .map(|r| {
                (
                    r[0].timestamp.as_deref().unwrap(),
                    r[4].timestamp.as_deref().unwrap(),
                )
            })
            .collect();
        for ((start, end), declared) in pairs.iter().zip(golden["stretches"].as_array().unwrap()) {
            assert_eq!(*start, declared["start"].as_str().unwrap());
            assert_eq!(*end, declared["end"].as_str().unwrap());
            assert_eq!(
                ms(end) - ms(start),
                declared["duration_ms"].as_i64().unwrap()
            );
        }
        let mut reversed = input.records.clone();
        reversed.reverse();
        db.store_mut()
            .upsert_records(&input.metadata.session_id, &reversed, keep)
            .unwrap();
        assert_eq!(serde_json::to_value(query(&db)).unwrap(), *expected);
        assert!(
            db.store()
                .records(&input.metadata.session_id)
                .unwrap()
                .iter()
                .all(|r| r.content_json.is_some() == keep)
        );
    }
}

#[test]
fn hands_off_last_tool_result_no_tool_zero_and_half_open_edges() {
    let mut db = TempDb::empty().unwrap();
    let mut rows = sample();
    rows.push(serde_json::from_value(json!({"uuid":"result","type":"user","timestamp":"2026-09-07T12:04:00Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"synthetic","content":"Synthetic"}]}})).unwrap());
    rows.extend([
        record("no-tool-h", "2026-09-07T12:05:00Z", true, false),
        record("no-tool-a", "2026-09-07T12:08:00Z", false, false),
        record("zero-h", "2026-09-07T12:10:00Z", true, false),
        record("zero-z", "2026-09-07T12:10:00Z", false, true),
        record("trailing", "2026-09-07T12:20:00Z", true, false),
        record("outside", "2026-09-08T00:00:00Z", false, true),
    ]);
    seed(&mut db, "s", "claude", Some("cli"), &rows, false);
    assert_eq!(
        (query(&db).n, query(&db).median_min, query(&db).p90_min),
        (Some(1), Some(4.0), Some(4.0))
    );
    let after_human = Window::new(ms("2026-09-07T12:00:01Z"), ms("2026-09-07T12:05:00Z")).unwrap();
    assert_eq!(query_in(&db, after_human).n, Some(0));
}

#[test]
fn hands_off_unknowns_and_positive_tool_evidence() {
    let mut db = TempDb::empty().unwrap();
    let mut rows = sample();
    rows.push(record("later", "2026-09-07T12:02:00Z", false, false));
    seed(&mut db, "s", "claude", None, &rows, false);
    let c = Connection::open(db.path()).unwrap();
    c.execute(
        "UPDATE records SET tool_use_count=NULL WHERE uuid='later'",
        [],
    )
    .unwrap();
    assert_eq!(query(&db).n, Some(1)); // Positive Read establishes tool eligibility.
    c.execute("UPDATE records SET tool_use_count=0 WHERE uuid='a'", [])
        .unwrap();
    let unknown = query(&db);
    assert_eq!(
        (unknown.n, unknown.median_min, unknown.p90_min),
        (None, None, None)
    );
    c.execute(
        "UPDATE records SET tool_use_count=0,is_human=NULL WHERE uuid='later'",
        [],
    )
    .unwrap();
    assert_eq!(query(&db).n, None);
    c.execute("UPDATE records SET is_human=0 WHERE uuid='later'", [])
        .unwrap();
    assert_eq!(query(&db).n, Some(0));
    // Missing time has no event-window membership, consistent with other metrics.
    let mut untimed = record("untimed", "2026-09-07T12:03:00Z", true, false);
    untimed.timestamp = None;
    seed(&mut db, "s", "claude", None, &[untimed], false);
    assert_eq!(query(&db).n, Some(0));
}

#[test]
fn hands_off_precise_order_and_nonpositive_leap_projection() {
    let mut db = TempDb::empty().unwrap();
    let rows = vec![
        record("z-human", "2026-09-07T12:00:00.0000000001Z", true, false),
        record("a-tool", "2026-09-07T12:00:00.0000000002Z", false, true),
        record("end", "2026-09-07T12:00:01Z", false, false),
    ];
    seed(&mut db, "precise", "claude", None, &rows, false);
    assert_eq!(query(&db).median_min, Some(1.0 / 60.0));
    let leap = [
        record("leap-human", "2016-12-31T23:59:60.9Z", true, false),
        record("ordinary-agent", "2017-01-01T00:00:00.1Z", false, true),
    ];
    seed(&mut db, "leap", "claude", None, &leap, false);
    let w = Window::new(ms("2016-12-31T23:59:59Z"), ms("2017-01-01T00:00:01Z")).unwrap();
    assert_eq!(query_in(&db, w).n, Some(0));
}

#[test]
fn hands_off_f9_threshold_matrix_and_healthy_sibling_surface() {
    let f = fixture("F9");
    let data = &f.snapshots()["hands_off"];
    for case in data["cases"].as_array().unwrap() {
        let mut db = TempDb::empty().unwrap();
        for name in case["sessions"].as_array().unwrap() {
            let s = data["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["session_id"] == *name)
                .unwrap();
            let rows: Vec<CanonicalRecord> = serde_json::from_value(s["records"].clone()).unwrap();
            seed(
                &mut db,
                name.as_str().unwrap(),
                "claude",
                Some("raw-batched"),
                &rows,
                false,
            );
        }
        let report = query(&db);
        assert_eq!(report.n, case["expected_n"].as_u64(), "{}", case["name"]);
        assert_eq!(
            report.excluded_surfaces.len(),
            usize::from(case["excluded"].as_bool().unwrap())
        );
        if let Some(excluded) = report.excluded_surfaces.first() {
            assert_eq!(excluded.host, "claude");
            assert_eq!(excluded.surface.as_deref(), Some("raw-batched"));
            assert_eq!(
                excluded.qualifying_sessions,
                case["qualifying_sessions"].as_u64().unwrap()
            );
            assert_eq!(
                excluded.degenerate_sessions,
                case["degenerate_sessions"].as_u64().unwrap()
            );
            assert_eq!((report.median_min, report.p90_min), (None, None));
            // Unknown classifications on an excluded surface cannot poison its sibling.
            Connection::open(db.path())
                .unwrap()
                .execute("UPDATE records SET is_human=NULL", [])
                .unwrap();
            seed(
                &mut db,
                "sibling",
                "claude",
                Some("raw-healthy"),
                &sample(),
                false,
            );
            let report = query(&db);
            assert_eq!((report.n, report.median_min), (Some(1), Some(1.0)));
        }
    }
}

#[test]
fn hands_off_health_uses_precise_equivalent_instants_and_session_floor() {
    for (label, timestamps, excluded) in [
        (
            "offset-equivalence",
            vec![
                "2026-09-07T12:00:00Z",
                "2026-09-07T05:00:00-07:00",
                "2026-09-07T12:00:00.000Z",
                "2026-09-07T12:01:00Z",
                "2026-09-07T05:01:00-07:00",
                "2026-09-07T12:01:00.0Z",
            ],
            true,
        ),
        (
            "submillisecond-distinct",
            vec![
                "2026-09-07T12:00:00.0000000001Z",
                "2026-09-07T12:00:00.0000000002Z",
                "2026-09-07T12:00:00.0000000003Z",
                "2026-09-07T12:00:00.0000000004Z",
                "2026-09-07T12:00:00.0000000005Z",
                "2026-09-07T12:01:00Z",
            ],
            false,
        ),
    ] {
        let mut db = TempDb::empty().unwrap();
        for i in 0..3 {
            let rows: Vec<_> = timestamps
                .iter()
                .enumerate()
                .map(|(j, ts)| record(&format!("{label}-{i}-{j}"), ts, j == 0, j == 1))
                .collect();
            seed(
                &mut db,
                &format!("{label}-{i}"),
                "claude",
                Some("raw-label"),
                &rows,
                false,
            );
        }
        assert_eq!(!query(&db).excluded_surfaces.is_empty(), excluded);
    }
    // Five records qualify, but only two such sessions cannot exclude a surface.
    // A third four-record session must not satisfy the qualifying-session floor.
    let mut db = TempDb::empty().unwrap();
    for (i, n) in [(0, 5), (1, 5), (2, 4)] {
        let rows: Vec<_> = (0..n)
            .map(|j| {
                record(
                    &format!("floor-{i}-{j}"),
                    "2026-09-07T12:00:00Z",
                    j == 0,
                    j == 1,
                )
            })
            .collect();
        seed(&mut db, &format!("floor-{i}"), "claude", None, &rows, false);
    }
    assert!(query(&db).excluded_surfaces.is_empty());
    seed(
        &mut db,
        "floor-2",
        "claude",
        None,
        &[record("floor-2-4", "2026-09-07T12:00:00Z", false, false)],
        false,
    );
    let report = query(&db);
    assert_eq!(report.excluded_surfaces[0].qualifying_sessions, 3);
    assert_eq!(report.excluded_surfaces[0].surface, None);
    // A literal raw label and another host are distinct from that NULL surface.
    seed(&mut db, "other-host", "codex", None, &sample(), false);
    let mut sibling = sample();
    for row in &mut sibling {
        row.uuid = Some(format!("sibling-{}", row.uuid.as_deref().unwrap()));
    }
    seed(
        &mut db,
        "other-surface",
        "claude",
        Some("unknown"),
        &sibling,
        false,
    );
    assert_eq!(query(&db).n, Some(2));
}

#[test]
fn hands_off_metadata_orders_empty_and_outside_unknowns() {
    let empty = HandsOff {
        n: Some(0),
        median_min: None,
        p90_min: None,
        excluded_surfaces: vec![],
    };
    assert_eq!(query(&TempDb::empty().unwrap()), empty);
    let mut baseline = None;
    for keep in [false, true] {
        for reverse in [false, true] {
            let mut db = TempDb::empty().unwrap();
            if keep {
                db.store_mut()
                    .set_retention_mode(RetentionMode::FullContent)
                    .unwrap();
            }
            let mut rows = sample();
            rows.push(record("outside", "2026-09-08T00:00:00Z", false, true));
            rows.push(record(
                "outside-leading",
                "2026-08-31T23:59:59Z",
                true,
                false,
            ));
            if reverse {
                rows.reverse();
            }
            seed(&mut db, "s", "claude", None, &rows, keep);
            let c = Connection::open(db.path()).unwrap();
            c.execute(
                "UPDATE records SET is_human=NULL WHERE uuid LIKE 'outside%'",
                [],
            )
            .unwrap();
            let report = query(&db);
            assert_eq!(report.n, Some(1));
            if let Some(expected) = &baseline {
                assert_eq!(&report, expected);
            } else {
                baseline = Some(report.clone());
            }
            assert!(
                db.store()
                    .records("s")
                    .unwrap()
                    .iter()
                    .all(|row| row.content_json.is_some() == keep)
            );
            assert!(
                db.store()
                    .records("s")
                    .unwrap()
                    .iter()
                    .flat_map(|row| &row.tool_uses)
                    .all(|tool| tool.input_json.is_some() == keep)
            );
            c.execute("INSERT INTO native_record_copies(session_id,record_uuid) SELECT session_id,uuid FROM records",[]).unwrap();
            assert_eq!(query(&db), report);
        }
    }
}

#[test]
fn hands_off_open_probes_actual_query_and_writer_restores_view() {
    for missing in ["surface", "type"] {
        let db = fixture("F1").build_db(false).unwrap();
        let c = Connection::open(db.path()).unwrap();
        // Retain every column required by the other metric APIs so this proves
        // the hands-off query itself is checked by the read-only open path.
        let columns = [
            "uuid",
            "session_id",
            "host",
            "surface",
            "ts_ms",
            "ts",
            "type",
            "role",
            "is_human",
            "text_len",
            "tool_use_count",
        ];
        let selected = columns
            .into_iter()
            .filter(|c| *c != missing)
            .collect::<Vec<_>>()
            .join(",");
        c.execute_batch(&format!("DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT {selected} FROM v_records WHERE ts_ms IS NOT NULL")).unwrap();
        let before: String = c
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='v_session_events'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(MetricsDb::open(db.path()).is_err(), "{missing}");
        let after: String = c
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='v_session_events'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(before, after);
        let _writer = xt_store::Store::open(db.path()).unwrap();
        assert_eq!(query(&db).n, Some(5));
    }
}

/// Only a message a person typed starts a stretch: hands-off reads the same
/// rule human messages count (M-02). A `claude -p` child launched by another
/// session has a prompt the structure calls an input, but the human-message
/// rule counts it as no person's, so it starts no stretch; its parent's
/// stretch is unchanged, and the Sessions column and the Dashboard agree.
#[test]
fn a_claude_print_childs_prompt_starts_no_stretch_and_its_parent_is_unchanged() {
    let mut db = TempDb::empty().unwrap();
    for (id, rows) in [
        (
            "parent",
            vec![
                record("p-h", "2026-09-07T12:00:00Z", true, false),
                record("p-a", "2026-09-07T12:03:00Z", false, true),
            ],
        ),
        (
            "child",
            vec![
                record("c-prompt", "2026-09-07T12:01:00Z", true, false),
                record("c-a1", "2026-09-07T12:02:00Z", false, true),
                record("c-a2", "2026-09-07T12:05:00Z", false, true),
            ],
        ),
    ] {
        let mut session = SessionMeta::new(id, "claude", SessionSource::Fixture);
        session.native_session_id = Some(format!("{id}-native"));
        db.store_mut().upsert_session(&session, false).unwrap();
        db.store_mut().upsert_records(id, &rows, false).unwrap();
    }
    let read = |db: &TempDb| {
        let metrics = MetricsDb::open(db.path()).unwrap();
        (
            metrics.hands_off(window()).unwrap(),
            metrics
                .session_hands_off(window(), &["parent", "child"])
                .unwrap(),
            metrics
                .counts(window(), xt_metrics::TypingRate::default())
                .unwrap()
                .human_messages,
        )
    };
    // Before anything says who launched it, the child's prompt is counted as
    // a person's message and starts a four-minute stretch.
    let (before, _, messages) = read(&db);
    assert_eq!((before.n, messages), (Some(2), Some(2)));
    // The child's recorded origin: the parent session launched it.
    db.store_mut()
        .import_human_session_origins(&xt_store::human_input::OriginManifest {
            version: 1,
            sessions: vec![xt_store::human_input::SessionOrigin {
                session_id: "child".into(),
                host: xt_store::Host::Claude,
                native_session_id: "child-native".into(),
                parent_host: xt_store::Host::Claude,
                parent_native_session_id: "parent-native".into(),
                method: "explicit_session_id".into(),
                evidence_id: "audit".into(),
                launch_id: "call".into(),
            }],
        })
        .unwrap();
    let (after, sessions, messages) = read(&db);
    assert_eq!(messages, Some(1));
    assert_eq!(after.n, Some(1));
    assert_eq!(after.median_min, Some(3.0));
    assert_eq!(
        sessions["parent"],
        xt_metrics::SessionHandsOff::Measured {
            n: 1,
            median_min: Some(3.0)
        }
    );
    assert_eq!(
        sessions["child"],
        xt_metrics::SessionHandsOff::Measured {
            n: 0,
            median_min: None
        }
    );
}
