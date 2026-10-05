use jiff::Timestamp;
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{MetricsDb, SessionStretch, SessionStretches, ToolBlock, Window};
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
fn query(db: &TempDb, session: &str) -> SessionStretches {
    MetricsDb::open(db.path())
        .unwrap()
        .session_stretches(window(), session)
        .unwrap()
}
fn measured(db: &TempDb, session: &str) -> Vec<SessionStretch> {
    match query(db, session) {
        SessionStretches::Measured { stretches } => stretches,
        other => panic!("expected a measured session, got {other:?}"),
    }
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
/// An assistant record whose content states tool calls at the named block
/// indices, with filler text in between so the indices are not all zero.
fn blocks(id: &str, ts: &str, tool_blocks: &[usize]) -> CanonicalRecord {
    let last = tool_blocks.iter().copied().max().unwrap();
    let content: Vec<_> = (0..=last)
        .map(|index| {
            if tool_blocks.contains(&index) {
                json!({"type":"tool_use","name":"Read","input":{"path":"synthetic"}})
            } else {
                json!({"type":"text","text":"Synthetic"})
            }
        })
        .collect();
    serde_json::from_value(
        json!({"uuid":id,"type":"assistant","timestamp":ts,"message":{"role":"assistant","content":content}}),
    )
    .unwrap()
}
/// The same median and probe p90 M-09 publishes, over durations collected one
/// session at a time. Stated here so the parity check cannot borrow the
/// implementation's own arithmetic.
fn median_p90(values: &mut [u64]) -> (f64, u64) {
    values.sort_unstable();
    let middle = values.len() / 2;
    let median = if values.len().is_multiple_of(2) {
        (values[middle - 1] as f64 + values[middle] as f64) / 2.0
    } else {
        values[middle] as f64
    };
    (median, values[values.len() - values.len().div_ceil(10)])
}

#[test]
fn session_stretches_f1_states_the_named_endpoints_locators_and_survives_copies() {
    let f = fixture("F1");
    let golden = f.snapshots()["hands_off"]["stretches"].clone();
    let declared = golden.as_array().unwrap();
    for keep in [false, true] {
        let db = f.build_db(keep).unwrap();
        let input = &f.sessions()[0];
        let id = &input.metadata.session_id;
        let stretches = measured(&db, id);
        assert_eq!(stretches.len(), declared.len());
        for ((stretch, chunk), declared) in stretches
            .iter()
            .zip(input.records.chunks(5))
            .zip(declared.iter())
        {
            // The independently declared endpoints, and the records that
            // actually carry them: a human start and a non-human end.
            assert_eq!(stretch.start, declared["start"].as_str().unwrap());
            assert_eq!(stretch.end, declared["end"].as_str().unwrap());
            assert_eq!(
                stretch.duration_ms,
                declared["duration_ms"].as_u64().unwrap()
            );
            assert_eq!(stretch.start_uuid, *chunk[0].uuid.as_ref().unwrap());
            assert_eq!(stretch.end_uuid, *chunk[4].uuid.as_ref().unwrap());
            // F1's only tool call of each stretch is the third record's sole
            // content block, whether or not that block's input was retained.
            assert_eq!(
                stretch.first_tool,
                Some(ToolBlock {
                    record_uuid: chunk[2].uuid.clone().unwrap(),
                    block_index: 0,
                })
            );
        }
        // Canonical copies do not multiply a session's stretches.
        let c = Connection::open(db.path()).unwrap();
        c.execute("INSERT INTO native_record_copies(session_id,record_uuid) SELECT session_id,uuid FROM records",[]).unwrap();
        assert_eq!(measured(&db, id), stretches);
        assert!(
            db.store()
                .records(id)
                .unwrap()
                .iter()
                .flat_map(|row| &row.tool_uses)
                .all(|tool| tool.input_json.is_some() == keep)
        );
        // An identifier no indexed user session owns is missing, and so is one
        // the shared session domain excludes.
        assert_eq!(query(&db, "absent"), SessionStretches::Missing);
        c.execute("UPDATE sessions SET kind='judge'", []).unwrap();
        assert_eq!(query(&db, id), SessionStretches::Missing);
    }
}

#[test]
fn session_stretches_wire_shape_names_its_state() {
    let db = TempDb::empty().unwrap();
    assert_eq!(
        serde_json::to_value(query(&db, "absent")).unwrap(),
        json!({"state": "missing"})
    );
    let mut db = db;
    seed(
        &mut db,
        "s",
        "claude",
        None,
        &[
            record("h", "2026-09-07T12:00:00Z", true, false),
            record("a", "2026-09-07T12:01:00Z", false, true),
        ],
        false,
    );
    assert_eq!(
        serde_json::to_value(query(&db, "s")).unwrap(),
        json!({"state":"measured","stretches":[{
            "start_uuid":"h","end_uuid":"a",
            "start":"2026-09-07T12:00:00Z","end":"2026-09-07T12:01:00Z",
            "duration_ms":60000,
            "first_tool":{"record_uuid":"a","block_index":0}
        }]})
    );
}

#[test]
fn session_stretches_order_multiple_and_drop_leading_agent_trailing_human_and_zero() {
    let mut db = TempDb::empty().unwrap();
    let rows = vec![
        // Nothing before the first human record starts a stretch.
        record("lead-agent", "2026-09-07T11:58:00Z", false, true),
        record("first-h", "2026-09-07T12:00:00Z", true, false),
        record("first-a", "2026-09-07T12:03:00Z", false, true),
        record("second-h", "2026-09-07T12:10:00Z", true, false),
        record("second-a", "2026-09-07T12:11:00Z", false, true),
        // A tool-result carrier still extends the stretch it belongs to.
        serde_json::from_value(json!({"uuid":"second-r","type":"user","timestamp":"2026-09-07T12:14:00Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"synthetic","content":"Synthetic"}]}})).unwrap(),
        // Same instant on both endpoints is not a positive duration.
        record("zero-h", "2026-09-07T12:20:00Z", true, false),
        record("zero-z", "2026-09-07T12:20:00Z", false, true),
        // An unanswered human record ends the session with no stretch.
        record("trailing-h", "2026-09-07T12:30:00Z", true, false),
    ];
    seed(&mut db, "s", "claude", Some("cli"), &rows, false);
    let stretches = measured(&db, "s");
    assert_eq!(
        stretches
            .iter()
            .map(|s| (s.start_uuid.as_str(), s.end_uuid.as_str(), s.duration_ms))
            .collect::<Vec<_>>(),
        vec![
            ("first-h", "first-a", 180_000),
            ("second-h", "second-r", 240_000),
        ]
    );
    // The half-open window ends before the first stretch's own end instant.
    let clipped = MetricsDb::open(db.path())
        .unwrap()
        .session_stretches(
            Window::new(ms("2026-09-07T12:00:00Z"), ms("2026-09-07T12:03:00Z")).unwrap(),
            "s",
        )
        .unwrap();
    assert_eq!(clipped, SessionStretches::Measured { stretches: vec![] });
    // An indexed session with no in-window record is an honest empty list,
    // never the missing state.
    seed(
        &mut db,
        "elsewhere",
        "claude",
        Some("cli"),
        &[
            record("old-h", "2026-08-01T12:00:00Z", true, false),
            record("old-a", "2026-08-01T12:01:00Z", false, true),
        ],
        false,
    );
    assert_eq!(
        query(&db, "elsewhere"),
        SessionStretches::Measured { stretches: vec![] }
    );
}

#[test]
fn session_stretches_own_unknowns_blank_it_but_another_session_s_never_do() {
    let mut db = TempDb::empty().unwrap();
    let rows = vec![
        record("h", "2026-09-07T12:00:00Z", true, false),
        record("a", "2026-09-07T12:01:00Z", false, true),
    ];
    seed(&mut db, "target", "claude", Some("cli"), &rows, false);
    let mut other = rows.clone();
    for row in &mut other {
        row.uuid = Some(format!("other-{}", row.uuid.as_deref().unwrap()));
    }
    seed(&mut db, "other", "claude", Some("cli"), &other, false);
    let c = Connection::open(db.path()).unwrap();

    // The global report cannot publish a distribution while any in-window
    // session is unmeasured; the untouched session is still measurable.
    c.execute("UPDATE records SET is_human=NULL WHERE uuid='other-a'", [])
        .unwrap();
    assert_eq!(
        MetricsDb::open(db.path())
            .unwrap()
            .hands_off(window())
            .unwrap()
            .n,
        None
    );
    assert_eq!(measured(&db, "target").len(), 1);
    assert_eq!(
        query(&db, "other"),
        SessionStretches::Unmeasured {
            excluded_surface: None
        }
    );

    // The target's own unknown classification moves its own boundaries.
    c.execute("UPDATE records SET is_human=NULL WHERE uuid='a'", [])
        .unwrap();
    assert_eq!(
        query(&db, "target"),
        SessionStretches::Unmeasured {
            excluded_surface: None
        }
    );
    c.execute("UPDATE records SET is_human=0 WHERE uuid='a'", [])
        .unwrap();
    // A positive stretch with no known tool call and an unknown tool count is
    // unmeasured too; a known positive count elsewhere establishes eligibility.
    c.execute(
        "UPDATE records SET tool_use_count=NULL WHERE uuid IN ('h','a')",
        [],
    )
    .unwrap();
    assert_eq!(
        query(&db, "target"),
        SessionStretches::Unmeasured {
            excluded_surface: None
        }
    );
    c.execute("UPDATE records SET tool_use_count=1 WHERE uuid='a'", [])
        .unwrap();
    assert_eq!(measured(&db, "target").len(), 1);
}

#[test]
fn session_stretches_name_the_surface_health_that_excluded_them_and_spare_other_hosts() {
    let f = fixture("F9");
    let data = &f.snapshots()["hands_off"];
    for case in data["cases"].as_array().unwrap() {
        let mut db = TempDb::empty().unwrap();
        let names: Vec<&str> = case["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
            .collect();
        for name in &names {
            let session = data["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["session_id"] == *name)
                .unwrap();
            let rows: Vec<CanonicalRecord> =
                serde_json::from_value(session["records"].clone()).unwrap();
            seed(&mut db, name, "claude", Some("raw-batched"), &rows, false);
        }
        // The same healthy session on another host, seeded in every case.
        seed(
            &mut db,
            "sibling",
            "codex",
            Some("raw-batched"),
            &[
                record("sib-h", "2026-09-07T12:00:00Z", true, false),
                record("sib-a", "2026-09-07T12:01:00Z", false, true),
            ],
            false,
        );
        // On the judged surface, but with nothing inside the window, so it
        // changes no count and cannot make its own surface healthy.
        seed(
            &mut db,
            "eventless",
            "claude",
            Some("raw-batched"),
            &[
                record("gone-h", "2026-08-01T12:00:00Z", true, false),
                record("gone-a", "2026-08-01T12:01:00Z", false, true),
            ],
            false,
        );
        let excluded = case["excluded"].as_bool().unwrap();
        match query(&db, "eventless") {
            SessionStretches::Unmeasured {
                excluded_surface: Some(surface),
            } => {
                assert!(excluded, "{}", case["name"]);
                assert_eq!(
                    surface.qualifying_sessions,
                    case["qualifying_sessions"].as_u64().unwrap()
                );
            }
            report => {
                assert!(!excluded, "{}: {report:?}", case["name"]);
                assert_eq!(report, SessionStretches::Measured { stretches: vec![] });
            }
        }
        for name in &names {
            let report = query(&db, name);
            if excluded {
                // Health is the whole raw surface's, judged over every session
                // it has in the window, and the reason says so.
                let SessionStretches::Unmeasured {
                    excluded_surface: Some(surface),
                } = report
                else {
                    panic!(
                        "{}: {name} should be excluded, got {report:?}",
                        case["name"]
                    );
                };
                assert_eq!(surface.host, "claude");
                assert_eq!(surface.surface.as_deref(), Some("raw-batched"));
                assert_eq!(
                    surface.qualifying_sessions,
                    case["qualifying_sessions"].as_u64().unwrap()
                );
                assert_eq!(
                    surface.degenerate_sessions,
                    case["degenerate_sessions"].as_u64().unwrap()
                );
            } else {
                assert_eq!(measured(&db, name).len(), 1, "{}", case["name"]);
                assert!(matches!(report, SessionStretches::Measured { .. }));
            }
        }
        // A different host is a different surface, healthy either way.
        assert_eq!(measured(&db, "sibling").len(), 1, "{}", case["name"]);
    }
}

#[test]
fn session_stretches_keep_a_null_raw_surface_separate_from_a_literal_label() {
    let mut db = TempDb::empty().unwrap();
    // Three sessions batched onto one instant, with no raw surface label.
    for index in 0..3 {
        let rows: Vec<_> = (0..6)
            .map(|block| {
                record(
                    &format!("null-{index}-{block}"),
                    "2026-09-07T12:00:00Z",
                    block == 0,
                    block == 1,
                )
            })
            .collect();
        seed(
            &mut db,
            &format!("null-{index}"),
            "claude",
            None,
            &rows,
            false,
        );
    }
    let report = query(&db, "null-0");
    let SessionStretches::Unmeasured {
        excluded_surface: Some(surface),
    } = report
    else {
        panic!("the unlabelled surface should be excluded, got {report:?}");
    };
    assert_eq!(surface.surface, None);
    assert_eq!(surface.qualifying_sessions, 3);
    assert_eq!(surface.degenerate_sessions, 3);
    // A literal label on the same host is its own surface and stays measured.
    seed(
        &mut db,
        "labelled",
        "claude",
        Some("unknown"),
        &[
            record("lab-h", "2026-09-07T12:00:00Z", true, false),
            record("lab-a", "2026-09-07T12:01:00Z", false, true),
        ],
        false,
    );
    assert_eq!(measured(&db, "labelled").len(), 1);
}

#[test]
fn session_stretches_locate_the_earliest_record_s_lowest_block_or_nothing_at_all() {
    let mut db = TempDb::empty().unwrap();
    let rows = vec![
        record("h", "2026-09-07T12:00:00Z", true, false),
        // Two calls in one record: the lowest block index is the first.
        blocks("first-agent", "2026-09-07T12:01:00Z", &[1, 3]),
        // A later record also calls tools, but it is not the earliest.
        blocks("later-agent", "2026-09-07T12:02:00Z", &[0]),
        record("end", "2026-09-07T12:03:00Z", false, false),
    ];
    seed(&mut db, "s", "claude", Some("cli"), &rows, false);
    let stretches = measured(&db, "s");
    assert_eq!(
        stretches[0].first_tool,
        Some(ToolBlock {
            record_uuid: "first-agent".into(),
            block_index: 1,
        })
    );
    assert_eq!(stretches[0].end_uuid, "end");

    // A stretch qualifies on the records' own tool counts. With the blocks
    // gone, the stretch stays and its locator is unknown, not invented.
    let c = Connection::open(db.path()).unwrap();
    c.execute("DELETE FROM tool_uses", []).unwrap();
    let stretches = measured(&db, "s");
    assert_eq!(stretches.len(), 1);
    assert_eq!(stretches[0].first_tool, None);
    // A block of another session is never borrowed for this one.
    seed(&mut db, "donor", "claude", Some("cli"), &rows, false);
    assert_eq!(measured(&db, "s")[0].first_tool, None);
}

#[test]
fn session_stretches_never_promote_a_later_call_when_an_earlier_one_is_unaccounted_for() {
    // The same stretch throughout: an earlier record stating two calls, then a
    // later record stating one. Whatever happens to the earlier record's
    // evidence, the later record's call is a different call and is never
    // reported as this stretch's first one.
    let rows = vec![
        record("h", "2026-09-07T12:00:00Z", true, false),
        blocks("first-agent", "2026-09-07T12:01:00Z", &[1, 3]),
        blocks("later-agent", "2026-09-07T12:02:00Z", &[0]),
        record("end", "2026-09-07T12:03:00Z", false, false),
    ];
    for (name, mutation) in [
        // Every block of the earliest calling record is missing.
        (
            "all blocks of the earliest calling record lost",
            "DELETE FROM tool_uses WHERE uuid='first-agent'",
        ),
        // Only its first block is missing: the lowest surviving index is 3,
        // but the call actually made first is the unstored one.
        (
            "the earliest block of that record lost",
            "DELETE FROM tool_uses WHERE uuid='first-agent' AND block_index=1",
        ),
        // A record before it states an unknown number of calls, so it cannot
        // be ruled out as the one that called first.
        (
            "an earlier record's call count unknown",
            "UPDATE records SET tool_use_count=NULL WHERE uuid='h'",
        ),
    ] {
        let mut db = TempDb::empty().unwrap();
        seed(&mut db, "s", "claude", Some("cli"), &rows, false);
        Connection::open(db.path())
            .unwrap()
            .execute(mutation, [])
            .unwrap();
        let stretches = measured(&db, "s");
        // The stretch itself, its duration and its eligibility are untouched:
        // only the locator becomes unknown.
        assert_eq!(stretches.len(), 1, "{name}");
        assert_eq!(stretches[0].start_uuid, "h", "{name}");
        assert_eq!(stretches[0].end_uuid, "end", "{name}");
        assert_eq!(stretches[0].duration_ms, 180_000, "{name}");
        assert_eq!(stretches[0].first_tool, None, "{name}");
    }

    // Control: with every stated call accounted for, the locator is known.
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, "s", "claude", Some("cli"), &rows, false);
    assert_eq!(
        measured(&db, "s")[0].first_tool,
        Some(ToolBlock {
            record_uuid: "first-agent".into(),
            block_index: 1,
        })
    );
    // A record stating no calls at all is simply passed over, and a later
    // record's unknown count cannot unsettle an already-established first call.
    Connection::open(db.path())
        .unwrap()
        .execute(
            "UPDATE records SET tool_use_count=NULL WHERE uuid='later-agent'",
            [],
        )
        .unwrap();
    assert_eq!(
        measured(&db, "s")[0].first_tool,
        Some(ToolBlock {
            record_uuid: "first-agent".into(),
            block_index: 1,
        })
    );
}

#[test]
fn session_stretches_keep_precise_endpoints_and_partition_the_m09_sample() {
    let mut db = TempDb::empty().unwrap();
    // Distinct durations across sessions, so the median and the probe's p90
    // both select rather than repeat a single value.
    let minutes = [[1_i64, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]];
    for (index, session) in minutes.iter().enumerate() {
        let mut rows = Vec::new();
        let mut at = 0_i64;
        for (turn, length) in session.iter().enumerate() {
            let start = format!("2026-09-0{}T{:02}:{:02}:00Z", index + 2, at / 60, at % 60);
            at += length;
            let end = format!("2026-09-0{}T{:02}:{:02}:00Z", index + 2, at / 60, at % 60);
            at += 30;
            rows.push(record(&format!("s{index}-h{turn}"), &start, true, false));
            rows.push(record(&format!("s{index}-a{turn}"), &end, false, true));
        }
        seed(
            &mut db,
            &format!("s{index}"),
            "claude",
            Some("cli"),
            &rows,
            false,
        );
    }
    let metrics = MetricsDb::open(db.path()).unwrap();
    let mut collected: Vec<u64> = (0..minutes.len())
        .flat_map(|index| measured(&db, &format!("s{index}")))
        .map(|stretch| stretch.duration_ms)
        .collect();
    let global = metrics.hands_off(window()).unwrap();
    assert_eq!(global.n, Some(collected.len() as u64));
    let (median, p90) = median_p90(&mut collected);
    assert_eq!(global.median_min, Some(median / 60000.0));
    assert_eq!(global.p90_min, Some(p90 as f64 / 60000.0));

    // Endpoints are the stored native spellings, sub-millisecond digits and
    // all; the duration stays the metric's millisecond projection.
    let precise = vec![
        record("z-human", "2026-09-07T12:00:00.0000000001Z", true, false),
        record("a-tool", "2026-09-07T12:00:00.0000000002Z", false, true),
        record("end", "2026-09-07T12:00:01Z", false, false),
    ];
    seed(&mut db, "precise", "claude", Some("cli"), &precise, false);
    let stretches = measured(&db, "precise");
    assert_eq!(stretches[0].start, "2026-09-07T12:00:00.0000000001Z");
    assert_eq!(stretches[0].end, "2026-09-07T12:00:01Z");
    assert_eq!(stretches[0].duration_ms, 1000);
    assert_eq!(stretches[0].start_uuid, "z-human");
}

#[test]
fn session_stretches_open_probes_the_actual_locator_query() {
    let db = fixture("F1").build_db(false).unwrap();
    let c = Connection::open(db.path()).unwrap();
    let before: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='tool_uses'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    c.execute_batch("ALTER TABLE tool_uses RENAME COLUMN block_index TO position")
        .unwrap();
    assert!(MetricsDb::open(db.path()).is_err());
    c.execute_batch("ALTER TABLE tool_uses RENAME COLUMN position TO block_index")
        .unwrap();
    assert_eq!(
        c.query_row(
            "SELECT sql FROM sqlite_master WHERE name='tool_uses'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
    assert!(MetricsDb::open(db.path()).is_ok());
}
