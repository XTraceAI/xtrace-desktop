//! M-20 over one session's M-09 stretches: what the metric counts, what it
//! refuses to count, and that it changes neither M-09 nor M-05.

use jiff::Timestamp;
use rusqlite::Connection;
use serde_json::{Value, json};
use xt_fixtures::TempDb;
use xt_metrics::{
    MetricsDb, RepeatDensity, RepeatGroup, RepeatThresholds, SessionRepeats, SessionStretches,
    StretchRepeats, ToolBlock, UnknownRepeats, Window,
};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource};

const SESSION: &str = "s";

fn ms(value: &str) -> i64 {
    value.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn report(db: &TempDb, session: &str, thresholds: RepeatThresholds) -> SessionRepeats {
    MetricsDb::open(db.path())
        .unwrap()
        .session_repeats(window(), session, thresholds)
        .unwrap()
}
fn measured(db: &TempDb, session: &str) -> Vec<StretchRepeats> {
    match report(db, session, RepeatThresholds::default()) {
        SessionRepeats::Measured { stretches, .. } => stretches,
        other => panic!("expected a measured session, got {other:?}"),
    }
}
fn only(db: &TempDb) -> StretchRepeats {
    let stretches = measured(db, SESSION);
    assert_eq!(stretches.len(), 1, "{stretches:?}");
    stretches.into_iter().next().unwrap()
}
fn density(db: &TempDb) -> RepeatDensity {
    only(db).repeats
}

/// A human record: it starts a stretch and states that it called nothing.
fn human(uuid: &str, ts: &str) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":uuid,"type":"user","timestamp":ts,
        "message":{"role":"user","content":[{"type":"text","text":"Synthetic"}]}}))
    .unwrap()
}
/// An assistant record whose content is exactly the supplied calls, each
/// `(tool name, input)`.
fn agent(uuid: &str, ts: &str, calls: &[(&str, Value)]) -> CanonicalRecord {
    let content: Vec<_> = calls
        .iter()
        .map(|(name, input)| json!({"type":"tool_use","name":name,"input":input}))
        .collect();
    serde_json::from_value(json!({"uuid":uuid,"type":"assistant","timestamp":ts,
        "message":{"role":"assistant","content":content}}))
    .unwrap()
}
/// Repeat one call `count` times inside one assistant record.
fn repeated(uuid: &str, ts: &str, name: &str, input: Value, count: usize) -> CanonicalRecord {
    let calls: Vec<_> = (0..count).map(|_| (name, input.clone())).collect();
    agent(uuid, ts, &calls)
}
fn seed(db: &mut TempDb, id: &str, rows: &[CanonicalRecord]) {
    let mut session = SessionMeta::new(id, "claude", SessionSource::Transcript);
    session.surface = Some("cli".into());
    db.store_mut().upsert_session(&session, false).unwrap();
    db.store_mut().upsert_records(id, rows, false).unwrap();
}
/// One stretch: a human record, one assistant record holding `calls`, and an
/// end four active minutes later unless another end is supplied.
fn one_stretch(calls: &[(&str, Value)], end: &str) -> Vec<CanonicalRecord> {
    vec![
        human("h", "2026-09-07T12:00:00Z"),
        agent("a", "2026-09-07T12:00:01Z", calls),
        agent("z", end, &[("Read", json!({"file_path": "/done"}))]),
    ]
}

#[test]
fn repeats_are_counted_by_the_stored_key_and_zero_is_a_measurement() {
    let mut db = TempDb::empty().unwrap();
    // Two calls of one path, one of another, and one call naming no
    // identifying argument at all: two groups repeat, one does not.
    seed(
        &mut db,
        SESSION,
        &one_stretch(
            &[
                ("Edit", json!({"file_path": "/a.rs"})),
                ("Edit", json!({"file_path": "/a.rs"})),
                ("Edit", json!({"file_path": "/b.rs"})),
                ("Bash", json!({"command": "cargo test"})),
                ("Bash", json!({"command": "cargo test"})),
                ("TodoWrite", json!({"todos": []})),
            ],
            "2026-09-07T12:04:01Z",
        ),
    );
    let RepeatDensity::Measured { repeats, worst } = density(&db) else {
        panic!("a complete stretch is measured");
    };
    // (2-1) for the repeated Edit, (2-1) for the repeated Bash. The second
    // Edit path, the marker call and the closing Read repeat nothing.
    assert_eq!(repeats, 2);
    assert_eq!(
        worst,
        Some(RepeatGroup {
            tool_name: "Edit".into(),
            count: 2,
            representative: ToolBlock {
                record_uuid: "a".into(),
                block_index: 0
            },
        })
    );

    // A stretch whose calls were all different measures zero, not unknown.
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        SESSION,
        &one_stretch(
            &[
                ("Edit", json!({"file_path": "/a.rs"})),
                ("Edit", json!({"file_path": "/b.rs"})),
                // The same string under another field, and under another tool,
                // are different calls.
                ("Edit", json!({"command": "/a.rs"})),
                ("Write", json!({"file_path": "/a.rs"})),
            ],
            "2026-09-07T12:04:01Z",
        ),
    );
    assert_eq!(
        density(&db),
        RepeatDensity::Measured {
            repeats: 0,
            worst: None
        }
    );
}

#[test]
fn missing_counts_blocks_keys_versions_or_a_contested_key_are_unknown_not_zero() {
    for (reason, damage) in [
        (
            UnknownRepeats::MissingBlocks,
            "DELETE FROM tool_uses WHERE uuid='a' AND block_index=2",
        ),
        (
            UnknownRepeats::MissingKey,
            "UPDATE tool_uses SET group_version=NULL,group_key=NULL WHERE uuid='a' AND block_index=2",
        ),
        (
            UnknownRepeats::UnsupportedVersion,
            "UPDATE tool_uses SET group_version=2 WHERE uuid='a' AND block_index=2",
        ),
        (
            UnknownRepeats::ConflictingKey,
            "UPDATE tool_uses SET group_conflict=1 WHERE uuid='a' AND block_index=2",
        ),
        // Evidence missing from an *earlier* record still blanks the stretch:
        // the calls it did not account for could have been the repeated ones.
        (
            UnknownRepeats::MissingKey,
            "UPDATE tool_uses SET group_version=NULL,group_key=NULL WHERE uuid='a' AND block_index=0",
        ),
        (
            UnknownRepeats::MissingBlocks,
            "DELETE FROM tool_uses WHERE uuid='a' AND block_index=0",
        ),
        // ... and so does one missing from the record that closes it.
        (
            UnknownRepeats::MissingKey,
            "UPDATE tool_uses SET group_version=NULL,group_key=NULL WHERE uuid='z'",
        ),
    ] {
        let mut db = TempDb::empty().unwrap();
        seed(
            &mut db,
            SESSION,
            &one_stretch(
                &[
                    ("Edit", json!({"file_path": "/a.rs"})),
                    ("Edit", json!({"file_path": "/a.rs"})),
                    ("Edit", json!({"file_path": "/a.rs"})),
                ],
                "2026-09-07T12:04:01Z",
            ),
        );
        // Complete, this stretch measures two repeats of one call.
        let before = only(&db);
        assert_eq!(
            before.repeats,
            RepeatDensity::Measured {
                repeats: 2,
                worst: Some(RepeatGroup {
                    tool_name: "Edit".into(),
                    count: 3,
                    representative: ToolBlock {
                        record_uuid: "a".into(),
                        block_index: 0
                    },
                })
            }
        );
        Connection::open(db.path())
            .unwrap()
            .execute(damage, [])
            .unwrap();
        let after = only(&db);
        assert_eq!(after.repeats, RepeatDensity::Unknown { reason }, "{damage}");
        // The stretch keeps its place, its endpoints and both durations; only
        // its repeat measurement and its circling judgement are unknown.
        assert_eq!(after.circling, None, "{damage}");
        assert_eq!(
            (
                after.start_uuid,
                after.end_uuid,
                after.duration_ms,
                after.active_duration_ms
            ),
            (
                before.start_uuid,
                before.end_uuid,
                before.duration_ms,
                before.active_duration_ms
            )
        );
    }

    // A record that never stated how many calls it made is its own unknown.
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        SESSION,
        &[
            human("h", "2026-09-07T12:00:00Z"),
            // No content at all: the source never said what this record did.
            serde_json::from_value(json!({"uuid":"quiet","type":"assistant",
                "timestamp":"2026-09-07T12:00:30Z","message":{"role":"assistant"}}))
            .unwrap(),
            agent(
                "a",
                "2026-09-07T12:04:01Z",
                &[
                    ("Edit", json!({"file_path": "/a.rs"})),
                    ("Edit", json!({"file_path": "/a.rs"})),
                ],
            ),
        ],
    );
    assert_eq!(
        density(&db),
        RepeatDensity::Unknown {
            reason: UnknownRepeats::CallCount
        }
    );
}

#[test]
fn circling_takes_both_thresholds_inclusively_and_active_time_is_not_elapsed_time() {
    // Exactly four active minutes and exactly five repeats: circling.
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        SESSION,
        &[
            human("h", "2026-09-07T12:00:00Z"),
            repeated(
                "a",
                "2026-09-07T12:02:00Z",
                "Edit",
                json!({"file_path": "/a.rs"}),
                6,
            ),
            agent(
                "z",
                "2026-09-07T12:04:00Z",
                &[("Read", json!({"file_path": "/done"}))],
            ),
        ],
    );
    let exact = only(&db);
    assert_eq!(
        (exact.duration_ms, exact.active_duration_ms),
        (240_000, 240_000)
    );
    assert_eq!(
        exact.repeats,
        RepeatDensity::Measured {
            repeats: 5,
            worst: Some(RepeatGroup {
                tool_name: "Edit".into(),
                count: 6,
                representative: ToolBlock {
                    record_uuid: "a".into(),
                    block_index: 0
                },
            })
        }
    );
    assert_eq!(exact.circling, Some(true));
    // One repeat short, or one millisecond short, and it is not.
    for thresholds in [
        RepeatThresholds::new(240_001, 5).unwrap(),
        RepeatThresholds::new(240_000, 6).unwrap(),
    ] {
        let SessionRepeats::Measured { stretches, .. } = report(&db, SESSION, thresholds) else {
            panic!("measured");
        };
        assert_eq!(stretches[0].circling, Some(false), "{thresholds:?}");
    }

    // The same calls with an idle hour in the middle: M-09's elapsed duration
    // includes the wait, M-05's fold does not, and the stretch is not circling
    // on active time even though it lasted over an hour.
    let mut idle = TempDb::empty().unwrap();
    seed(
        &mut idle,
        SESSION,
        &[
            human("h", "2026-09-07T12:00:00Z"),
            repeated(
                "a",
                "2026-09-07T12:01:00Z",
                "Edit",
                json!({"file_path": "/a.rs"}),
                6,
            ),
            agent(
                "z",
                "2026-09-07T13:01:30Z",
                &[("Read", json!({"file_path": "/done"}))],
            ),
        ],
    );
    let waited = only(&idle);
    assert_eq!(waited.duration_ms, 3_690_000);
    // Only the first minute is active; the 60.5-minute gap exceeds M-05's
    // twenty minutes and contributes nothing.
    assert_eq!(waited.active_duration_ms, 60_000);
    assert!(matches!(
        waited.repeats,
        RepeatDensity::Measured { repeats: 5, .. }
    ));
    assert_eq!(waited.circling, Some(false));
}

#[test]
fn the_worst_group_breaks_ties_by_position_and_a_copied_record_is_counted_once() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        SESSION,
        &[
            human("h", "2026-09-07T12:00:00Z"),
            agent(
                "a",
                "2026-09-07T12:01:00Z",
                &[
                    // Three groups of two. The first one in the stretch wins.
                    ("Grep", json!({"pattern": "second"})),
                    ("Edit", json!({"file_path": "/first.rs"})),
                    ("Edit", json!({"file_path": "/first.rs"})),
                    ("Grep", json!({"pattern": "second"})),
                ],
            ),
            agent(
                "z",
                "2026-09-07T12:02:00Z",
                &[
                    ("Bash", json!({"command": "third"})),
                    ("Bash", json!({"command": "third"})),
                ],
            ),
        ],
    );
    let expected = RepeatGroup {
        tool_name: "Grep".into(),
        count: 2,
        representative: ToolBlock {
            record_uuid: "a".into(),
            block_index: 0,
        },
    };
    let RepeatDensity::Measured { repeats, worst } = density(&db) else {
        panic!("measured");
    };
    assert_eq!(repeats, 3);
    assert_eq!(worst, Some(expected.clone()));
    // The same answer on every read: the tie is broken by position, not by how
    // the opaque keys happen to sort.
    for _ in 0..3 {
        assert_eq!(
            density(&db),
            RepeatDensity::Measured {
                repeats,
                worst: worst.clone()
            }
        );
    }

    // A record another native context also carries is one call, not two.
    let before = only(&db);
    Connection::open(db.path())
        .unwrap()
        .execute(
            "INSERT INTO native_record_copies(session_id,record_uuid)
             SELECT session_id,uuid FROM records",
            [],
        )
        .unwrap();
    assert_eq!(only(&db), before);
}

#[test]
fn the_report_states_its_thresholds_its_states_and_never_a_key() {
    let db = TempDb::empty().unwrap();
    assert_eq!(
        serde_json::to_value(report(&db, "absent", RepeatThresholds::default())).unwrap(),
        json!({"state": "missing"})
    );
    let mut db = db;
    seed(
        &mut db,
        SESSION,
        &[
            human("h", "2026-09-07T12:00:00Z"),
            repeated(
                "a",
                "2026-09-07T12:04:00Z",
                "Edit",
                json!({"file_path": "/a.rs"}),
                6,
            ),
        ],
    );
    let wire = serde_json::to_value(report(&db, SESSION, RepeatThresholds::default())).unwrap();
    assert_eq!(
        wire,
        json!({"state":"measured","thresholds":{"active_ms":240_000,"repeats":5},
            "stretches":[{
                "start_uuid":"h","end_uuid":"a",
                "start":"2026-09-07T12:00:00Z","end":"2026-09-07T12:04:00Z",
                "duration_ms":240_000,"active_duration_ms":240_000,
                "repeats":{"state":"measured","repeats":5,"worst":{
                    "tool_name":"Edit","count":6,
                    "representative":{"record_uuid":"a","block_index":0}}},
                "circling":true}]})
    );
    // No comparison key, under any name, reaches the wire: the stored digests
    // of this session appear nowhere in it.
    let text = wire.to_string();
    let keys: Vec<String> = Connection::open(db.path())
        .unwrap()
        .prepare("SELECT DISTINCT group_key FROM tool_uses WHERE group_key IS NOT NULL")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(!keys.is_empty());
    for key in keys {
        assert!(!text.contains(&key));
    }
    for forbidden in [
        "group_key",
        "digest",
        "sha",
        "command",
        "file_path",
        "/a.rs",
    ] {
        assert!(!text.contains(forbidden), "{forbidden}");
    }
}

#[test]
fn m20_reuses_m09_s_membership_and_health_and_changes_neither_m09_nor_m05() {
    let mut db = TempDb::empty().unwrap();
    let rows = vec![
        human("h", "2026-09-07T12:00:00Z"),
        repeated(
            "a",
            "2026-09-07T12:03:00Z",
            "Edit",
            json!({"file_path": "/a.rs"}),
            3,
        ),
        human("h2", "2026-09-07T12:15:00Z"),
        agent(
            "a2",
            "2026-09-07T12:16:00Z",
            &[("Read", json!({"file_path": "/b.rs"}))],
        ),
    ];
    seed(&mut db, SESSION, &rows);
    let metrics = MetricsDb::open(db.path()).unwrap();
    let hands_off = metrics.hands_off(window()).unwrap();
    let spans = metrics.active_spans(window()).unwrap();
    let stretches = match metrics.session_stretches(window(), SESSION).unwrap() {
        SessionStretches::Measured { stretches } => stretches,
        other => panic!("{other:?}"),
    };
    let repeats = measured(&db, SESSION);

    // The same stretches, in the same order, with the same M-09 durations.
    assert_eq!(repeats.len(), stretches.len());
    for (repeat, stretch) in repeats.iter().zip(&stretches) {
        assert_eq!(repeat.start_uuid, stretch.start_uuid);
        assert_eq!(repeat.end_uuid, stretch.end_uuid);
        assert_eq!(repeat.start, stretch.start);
        assert_eq!(repeat.end, stretch.end);
        assert_eq!(repeat.duration_ms, stretch.duration_ms);
    }
    // Active time is its own measurement, folded over the records a stretch
    // selects. The twelve minutes the user spent between the two stretches are
    // active time of the session and belong to neither stretch, so the
    // session's M-05 total is far more than the stretches add up to.
    assert_eq!(
        repeats
            .iter()
            .map(|r| (r.duration_ms, r.active_duration_ms))
            .collect::<Vec<_>>(),
        vec![(180_000, 180_000), (60_000, 60_000)]
    );
    assert_eq!(spans.active_ms, 960_000);
    assert_eq!(
        repeats.iter().map(|r| r.active_duration_ms).sum::<u64>(),
        240_000
    );

    // Reading M-20 changes nothing either metric reports.
    assert_eq!(metrics.hands_off(window()).unwrap(), hands_off);
    assert_eq!(metrics.active_spans(window()).unwrap(), spans);
    assert_eq!(
        metrics.session_stretches(window(), SESSION).unwrap(),
        SessionStretches::Measured { stretches }
    );

    // Membership and the surface's clock are M-09's own answers. An indexed
    // session with no in-window record is an honest empty list ...
    seed(
        &mut db,
        "elsewhere",
        &[
            human("old-h", "2026-08-01T12:00:00Z"),
            agent(
                "old-a",
                "2026-08-01T12:01:00Z",
                &[("Read", json!({"file_path": "/a"}))],
            ),
        ],
    );
    assert!(matches!(
        report(&db, "elsewhere", RepeatThresholds::default()),
        SessionRepeats::Measured { ref stretches, .. } if stretches.is_empty()
    ));
    // ... and a session the shared session domain excludes is missing.
    Connection::open(db.path())
        .unwrap()
        .execute("UPDATE sessions SET kind='judge'", [])
        .unwrap();
    assert_eq!(
        report(&db, SESSION, RepeatThresholds::default()),
        SessionRepeats::Missing
    );
}

#[test]
fn a_surface_whose_clock_m09_excludes_reports_no_repeats_for_its_sessions() {
    let mut db = TempDb::empty().unwrap();
    // Three qualifying sessions on one surface, all degenerate: M-09 excludes
    // the surface, so M-20 has no stretch to count calls inside.
    for index in 0..3 {
        let rows: Vec<_> = (0..6)
            .map(|n| {
                let uuid = format!("s{index}-r{n}");
                if n % 2 == 0 {
                    human(&uuid, "2026-09-07T12:00:00Z")
                } else {
                    agent(
                        &uuid,
                        "2026-09-07T12:00:00Z",
                        &[("Edit", json!({"file_path": "/a.rs"}))],
                    )
                }
            })
            .collect();
        seed(&mut db, &format!("degenerate-{index}"), &rows);
    }
    let SessionRepeats::Unmeasured { excluded_surface } =
        report(&db, "degenerate-0", RepeatThresholds::default())
    else {
        panic!("an excluded surface is unmeasured");
    };
    let excluded = excluded_surface.expect("the surface is named");
    assert_eq!(excluded.surface.as_deref(), Some("cli"));
    assert_eq!(
        (excluded.qualifying_sessions, excluded.degenerate_sessions),
        (3, 3)
    );
}

/// Write `first`, measure it, then replay `again` for the same record UUIDs
/// through the store's own writer and measure once more.
fn replayed(
    first: &[CanonicalRecord],
    again: &[CanonicalRecord],
) -> (RepeatDensity, RepeatDensity) {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, SESSION, first);
    let before = density(&db);
    db.store_mut()
        .upsert_records(SESSION, again, false)
        .unwrap();
    (before, density(&db))
}

#[test]
fn a_replay_that_changes_which_call_sits_at_a_position_is_not_measured() {
    let edits = |uuid, name: &str| {
        repeated(
            uuid,
            "2026-09-07T12:02:00Z",
            name,
            json!({"file_path": "/a.rs"}),
            3,
        )
    };
    let stretch = |calls: CanonicalRecord| {
        vec![
            human("h", "2026-09-07T12:00:00Z"),
            calls,
            agent(
                "z",
                "2026-09-07T12:04:00Z",
                &[("Read", json!({"file_path": "/done"}))],
            ),
        ]
    };
    let measured = RepeatDensity::Measured {
        repeats: 2,
        worst: Some(RepeatGroup {
            tool_name: "Edit".into(),
            count: 3,
            representative: ToolBlock {
                record_uuid: "a".into(),
                block_index: 0,
            },
        }),
    };
    let disputed = RepeatDensity::Unknown {
        reason: UnknownRepeats::ConflictingKey,
    };

    // The same record and block indices now name another tool. The stored keys
    // were derived for the old name; counting with them would report repeats
    // of a call this position is no longer agreed to hold.
    let (before, after) = replayed(&stretch(edits("a", "Edit")), &[edits("a", "Write")]);
    assert_eq!(before, measured);
    assert_eq!(after, disputed);

    // An earlier text block moves every call one position, and the call that
    // now sits at block 2 is a different one. Paired by array order this was
    // invisible and the stretch still measured two repeats.
    let shifted: CanonicalRecord = serde_json::from_value(json!({
    "uuid":"a","type":"assistant","timestamp":"2026-09-07T12:02:00Z",
    "message":{"role":"assistant","content":[
        {"type":"text","text":"Synthetic"},
        {"type":"tool_use","name":"Edit","input":{"file_path":"/a.rs"}},
        {"type":"tool_use","name":"Edit","input":{"file_path":"/a.rs"}},
        {"type":"tool_use","name":"Edit","input":{"file_path":"/other.rs"}}
    ]}}))
    .unwrap();
    let (before, after) = replayed(&stretch(edits("a", "Edit")), &[shifted]);
    assert_eq!(before, measured);
    assert_eq!(after, disputed);

    // The same calls observed again leave the measurement exactly as it was.
    let (before, after) = replayed(&stretch(edits("a", "Edit")), &[edits("a", "Edit")]);
    assert_eq!((before, after), (measured.clone(), measured));
}
