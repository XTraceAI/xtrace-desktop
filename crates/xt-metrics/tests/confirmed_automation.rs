//! A confirmed automated input is neutral to every human-derived metric and
//! invisible to none of the agent work it caused.
//!
//! Each adversarial sequence is measured three ways where that proves
//! something: unconfirmed (the input still counts as human, as before), with
//! the structural confirmation, and — where the rule is easy to get wrong —
//! with the naive shortcut of storing the input as non-human, which every fold
//! would read as agent work. The confirmation must differ from both where the
//! contract says it does, and match the unconfirmed agent-side measurements
//! exactly.

mod hands_off_support;

use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use xt_fixtures::TempDb;
use xt_metrics::{
    MetricsDb, RepeatThresholds, SessionHandsOff, SessionStretches, SessionWindow, TypingRate,
    Window,
};
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource,
    confirmation::{AutomatedInputProof, ConfirmationDisposition, EvidenceKind},
};

fn ms(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn at(minute: u32) -> String {
    format!("2026-09-07T12:{minute:02}:00Z")
}
fn record(value: Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
/// A user text input. Whether a person or an agent submitted it is exactly
/// what the stored row cannot say; only a confirmation can.
fn input(uuid: &str, minute: u32, text: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user","timestamp":at(minute),
        "message":{"role":"user","content":[{"type":"text","text":text}]}}))
}
fn tool(uuid: &str, minute: u32, model: &str) -> CanonicalRecord {
    record(
        json!({"uuid":uuid,"type":"assistant","timestamp":at(minute),
        "message":{"id":format!("msg-{uuid}"),"role":"assistant","model":model,
            "content":[{"type":"tool_use","name":"Read","input":{"path":format!("synthetic/{uuid}")}}],
            "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":1,"cache_creation_input_tokens":2}}}),
    )
}
fn result(uuid: &str, minute: u32) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user","timestamp":at(minute),
        "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"synthetic"}]}}))
}
fn reply(uuid: &str, minute: u32, model: &str) -> CanonicalRecord {
    record(
        json!({"uuid":uuid,"type":"assistant","timestamp":at(minute),
        "message":{"id":format!("msg-{uuid}"),"role":"assistant","model":model,
            "content":[{"type":"text","text":"Synthetic reply"}],
            "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":1,"cache_creation_input_tokens":2}}}),
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Treatment {
    /// No confirmation: the input still counts as human, as before.
    Unconfirmed,
    /// The structural confirmation this change adds.
    Confirmed,
    /// The wrong shortcut: rewrite the stored classification to non-human.
    NaiveNonHuman,
}

fn seed(db: &mut TempDb, session: &str, rows: &[CanonicalRecord]) {
    let mut meta = SessionMeta::new(session, "claude", SessionSource::Transcript);
    meta.native_session_id = Some(session.into());
    meta.surface = Some("cli".into());
    db.store_mut().upsert_session(&meta, false).unwrap();
    db.store_mut().upsert_records(session, rows, false).unwrap();
}

fn treat(db: &mut TempDb, session: &str, automated: &[&str], treatment: Treatment) {
    match treatment {
        Treatment::Unconfirmed => {}
        Treatment::Confirmed => {
            let proofs: Vec<_> = automated
                .iter()
                .enumerate()
                .map(|(index, uuid)| AutomatedInputProof {
                    record_uuid: (*uuid).into(),
                    session_id: session.into(),
                    native_session_id: session.into(),
                    parent_host: Host::Codex,
                    parent_session_id: "codex-synthetic-parent".into(),
                    parent_tool_call_id: format!("call-{session}-{index}"),
                    parent_operation_index: 0,
                    parent_result_id: None,
                    evidence_kind: EvidenceKind::AgentDispatch,
                    matcher_version: 1,
                })
                .collect();
            let report = db
                .store_mut()
                .apply_automated_input_confirmations(&proofs, 1)
                .unwrap();
            assert!(
                report
                    .dispositions
                    .iter()
                    .all(|d| *d == ConfirmationDisposition::Confirmed)
            );
        }
        Treatment::NaiveNonHuman => {
            let sql = Connection::open(db.path()).unwrap();
            for uuid in automated {
                sql.execute("UPDATE records SET is_human=0 WHERE uuid=?1", [uuid])
                    .unwrap();
            }
        }
    }
}

fn build(sessions: &[(&str, Vec<CanonicalRecord>, Vec<&str>)], treatment: Treatment) -> TempDb {
    let mut db = TempDb::empty().unwrap();
    for (session, rows, automated) in sessions {
        seed(&mut db, session, rows);
        treat(&mut db, session, automated, treatment);
    }
    db
}

fn metrics(db: &TempDb) -> MetricsDb {
    MetricsDb::open(db.path()).unwrap()
}
fn human_minutes(db: &TempDb) -> Option<f64> {
    metrics(db)
        .human_time(window(), TypingRate::default(), TimeZone::UTC)
        .unwrap()
        .human_minutes_est
}
fn stretches(db: &TempDb, session: &str) -> Value {
    serde_json::to_value(metrics(db).session_stretches(window(), session).unwrap()).unwrap()
}

#[test]
fn m07_counts_characters_independently_of_agent_gaps() {
    // Agent at 0, automated input at 5, human at 10.
    let rows = vec![
        reply("agent", 0, "synthetic-model"),
        input(
            "automated",
            5,
            "Synthetic dispatched request of some length",
        ),
        input("human", 10, "Synthetic human"),
    ];
    let sessions = [("s", rows, vec!["automated"])];
    let unconfirmed = build(&sessions, Treatment::Unconfirmed);
    let confirmed = build(&sessions, Treatment::Confirmed);
    let naive = build(&sessions, Treatment::NaiveNonHuman);

    // Both exclude the dispatched characters; timings do not enter the estimate.
    assert_eq!(human_minutes(&confirmed), Some(15.0 / 200.0));
    assert_eq!(human_minutes(&naive), Some(15.0 / 200.0));
    // Before correction, both complete input lengths count.
    assert_eq!(
        human_minutes(&unconfirmed),
        Some((15 + "Synthetic dispatched request of some length".len()) as f64 / 200.0)
    );

    let rate = TypingRate::default();
    let before = metrics(&unconfirmed).counts(window(), rate).unwrap();
    let after = metrics(&confirmed).counts(window(), rate).unwrap();
    assert_eq!(
        (before.human_messages, after.human_messages),
        (Some(2), Some(1))
    );
    assert_eq!(after.human_chars, Some("Synthetic human".len() as u64));
    assert_eq!(before.sessions, after.sessions);
    assert_eq!(before.assistant_records, after.assistant_records);
    assert_eq!(before.tool_calls, after.tool_calls);
}

#[test]
fn m07_typing_estimate_and_ratio_are_unaffected_by_a_leading_confirmed_input() {
    // No agent event precedes the human: the typing estimate applies to the
    // human only; the automated input contributes no characters or interval.
    let rows = vec![
        input("automated", 0, &"x".repeat(400)),
        input("human", 10, &"y".repeat(200)),
        reply("agent", 11, "synthetic-model"),
    ];
    let sessions = [("s", rows, vec!["automated"])];
    let confirmed = build(&sessions, Treatment::Confirmed);
    let unconfirmed = build(&sessions, Treatment::Unconfirmed);
    // 200 characters at 200 cpm.
    assert_eq!(human_minutes(&confirmed), Some(1.0));
    // Unconfirmed: two minutes typing the first input, one for the second.
    assert_eq!(human_minutes(&unconfirmed), Some(3.0));
    let report = metrics(&confirmed)
        .human_time(window(), TypingRate::default(), TimeZone::UTC)
        .unwrap();
    // M-05 keeps the all-event timeline: 0 → 11 minutes of activity.
    assert_eq!(report.agent_minutes, 11.0);
}

#[test]
fn a_confirmed_input_after_a_human_is_no_response_and_no_stretch_endpoint() {
    // Human then automated input, no agent: no M-03 turn.
    let alone = [(
        "s",
        vec![
            input("human", 0, "Synthetic human"),
            input("automated", 2, "Synthetic dispatched"),
        ],
        vec!["automated"],
    )];
    let rate = TypingRate::default();
    for (treatment, turns) in [
        (Treatment::Confirmed, Some(0)),
        (Treatment::NaiveNonHuman, Some(1)),
    ] {
        let db = build(&alone, treatment);
        assert_eq!(
            metrics(&db).counts(window(), rate).unwrap().assistant_turns,
            turns
        );
    }

    // Human, tool call, tool result, then an automated input: the stretch
    // ends at the tool result, never at the input.
    let trailing = [(
        "s",
        vec![
            input("human", 0, "Synthetic human"),
            tool("call", 1, "synthetic-model"),
            result("result", 2),
            input("automated", 5, "Synthetic dispatched"),
        ],
        vec!["automated"],
    )];
    let confirmed = build(&trailing, Treatment::Confirmed);
    let listed = stretches(&confirmed, "s");
    assert_eq!(listed["state"], "measured");
    let only = &listed["stretches"].as_array().unwrap()[..];
    assert_eq!(only.len(), 1);
    assert_eq!(only[0]["end_uuid"], "result");
    assert_eq!(only[0]["duration_ms"], 120_000);
    let naive = build(&trailing, Treatment::NaiveNonHuman);
    assert_eq!(
        stretches(&naive, "s")["stretches"][0]["end_uuid"],
        "automated"
    );
    assert_eq!(
        metrics(&confirmed)
            .counts(window(), rate)
            .unwrap()
            .assistant_turns,
        Some(1)
    );
}

#[test]
fn human_tool_automated_tool_human_keeps_one_uninterrupted_stretch() {
    let rows = vec![
        input("human", 0, "Synthetic human"),
        tool("first", 1, "synthetic-model"),
        result("first-result", 2),
        input("automated", 3, "Synthetic dispatched"),
        tool("second", 4, "synthetic-model"),
        result("second-result", 5),
        input("next-human", 10, "Synthetic human again"),
    ];
    let sessions = [("s", rows, vec!["automated"])];
    let unconfirmed = build(&sessions, Treatment::Unconfirmed);
    let confirmed = build(&sessions, Treatment::Confirmed);

    let before = stretches(&unconfirmed, "s");
    assert_eq!(before["stretches"].as_array().unwrap().len(), 2);
    let after = stretches(&confirmed, "s");
    assert_eq!(
        after,
        json!({"state":"measured","stretches":[{
            "start_uuid":"human","end_uuid":"second-result",
            "start":at(0),"end":at(5),"duration_ms":300_000,
            "first_tool":{"record_uuid":"first","block_index":0}
        }]})
    );
    let m = metrics(&confirmed);
    let (hands_off, days) = hands_off_support::matches_standalone(&m, window(), TimeZone::UTC);
    assert_eq!((hands_off.n, hands_off.median_min), (Some(1), Some(5.0)));
    // The neutral input starts no day's stretch either.
    assert_eq!(
        days.iter().map(|day| day.n).collect::<Vec<_>>(),
        [Some(0); 6]
            .into_iter()
            .chain([Some(1)])
            .collect::<Vec<_>>()
    );
    assert_eq!(
        m.session_hands_off(window(), &["s"]).unwrap()["s"],
        SessionHandsOff::Measured {
            n: 1,
            median_min: Some(5.0)
        }
    );
    // The circling view walks the same stretch: two calls, no repeats, and the
    // neutral input neither adds a record nor makes its call count unknown.
    let repeats = serde_json::to_value(
        m.session_repeats(window(), "s", RepeatThresholds::default())
            .unwrap(),
    )
    .unwrap();
    let stretch = &repeats["stretches"][0];
    assert_eq!(stretch["start_uuid"], "human");
    assert_eq!(stretch["end_uuid"], "second-result");
    assert_eq!(stretch["duration_ms"], 300_000);
    assert_eq!(stretch["active_duration_ms"], 300_000);
    assert_eq!(stretch["repeats"]["state"], "measured", "{stretch}");
    // One turn: the automated input neither closes nor opens one.
    let rate = TypingRate::default();
    assert_eq!(
        metrics(&unconfirmed)
            .counts(window(), rate)
            .unwrap()
            .assistant_turns,
        Some(2)
    );
    assert_eq!(m.counts(window(), rate).unwrap().assistant_turns, Some(1));
    // M-05 is unchanged by the correction.
    assert_eq!(
        metrics(&unconfirmed).active_spans(window()).unwrap(),
        m.active_spans(window()).unwrap()
    );
}

#[test]
fn an_automated_only_session_keeps_its_work_but_no_invented_human() {
    let rows = vec![
        input("automated", 0, "Synthetic dispatched"),
        tool("call", 1, "synthetic-model"),
        result("result", 2),
        reply("reply", 3, "synthetic-model"),
    ];
    let sessions = [("s", rows, vec!["automated"])];
    let unconfirmed = build(&sessions, Treatment::Unconfirmed);
    let confirmed = build(&sessions, Treatment::Confirmed);
    let (before, after) = (metrics(&unconfirmed), metrics(&confirmed));
    let rate = TypingRate::default();
    let counts = after.counts(window(), rate).unwrap();
    assert_eq!(counts.human_messages, Some(0));
    assert_eq!(counts.human_chars, Some(0));
    assert_eq!(counts.assistant_turns, Some(0));
    assert_eq!(counts.sessions, 1);
    assert_eq!(counts.assistant_records, 2);
    assert_eq!(counts.tool_calls, Some(1));
    assert_eq!(
        after
            .human_time(window(), rate, TimeZone::UTC)
            .unwrap()
            .human_minutes_est,
        Some(0.0)
    );
    // No human-seeded stretch exists, and none is invented from the input.
    assert_eq!(after.hands_off(window()).unwrap().n, Some(0));
    assert_eq!(
        stretches(&confirmed, "s"),
        json!({"state":"measured","stretches":[]})
    );
    assert_eq!(
        stretches(&unconfirmed, "s")["stretches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Tokens, activity and per-session work are identical to before.
    assert_eq!(
        before.tokens(window(), TimeZone::UTC).unwrap(),
        after.tokens(window(), TimeZone::UTC).unwrap()
    );
    assert_eq!(
        before.active_spans(window()).unwrap(),
        after.active_spans(window()).unwrap()
    );
    assert_eq!(
        before.concurrency(window()).unwrap(),
        after.concurrency(window()).unwrap()
    );
    let SessionWindow::Indexed {
        events,
        human_messages,
        tool_calls,
        agent_ms,
        ..
    } = after.session_windows(window(), &["s"]).unwrap()["s"].clone()
    else {
        panic!("indexed")
    };
    assert_eq!(
        (events, human_messages, tool_calls, agent_ms),
        (4, Some(0), Some(1), 180_000)
    );
}

#[test]
fn unknown_classification_stays_unknown_beside_a_confirmed_input() {
    let rows = vec![
        input("automated", 0, "Synthetic dispatched"),
        record(json!({"uuid":"unknown","type":"user","timestamp":at(1),"message":{"role":"user"}})),
        reply("reply", 2, "synthetic-model"),
    ];
    let db = build(&[("s", rows, vec!["automated"])], Treatment::Confirmed);
    let m = metrics(&db);
    let rate = TypingRate::default();
    let counts = m.counts(window(), rate).unwrap();
    assert_eq!(counts.human_messages, None);
    assert_eq!(counts.assistant_turns, None);
    assert_eq!(
        m.human_time(window(), rate, TimeZone::UTC)
            .unwrap()
            .human_minutes_est,
        None
    );
    let (hands_off, days) = hands_off_support::matches_standalone(&m, window(), TimeZone::UTC);
    assert_eq!(hands_off.n, None);
    // Only the day the unknown session touches is unknown.
    assert_eq!(days.iter().filter(|day| day.n.is_none()).count(), 1);
    assert_eq!(days[6].n, None);
    assert_eq!(
        stretches(&db, "s"),
        json!({"state":"unmeasured","excluded_surface":null})
    );
}

#[test]
fn favorite_turns_use_the_same_boundaries() {
    // Two models tie on output. Turns decide: one turn, attributed to its
    // first assistant record's model, because the automated input between
    // them is no boundary. Counting it as a boundary would give each model
    // a turn, and the lexical tie-break would pick the other one.
    let rows = vec![
        input("human", 0, "Synthetic human"),
        reply("first", 1, "zeta-model"),
        input("automated", 2, "Synthetic dispatched"),
        reply("second", 3, "alpha-model"),
    ];
    let sessions = [("s", rows, vec!["automated"])];
    for (treatment, favorite) in [
        (Treatment::Confirmed, "zeta-model"),
        (Treatment::Unconfirmed, "alpha-model"),
        (Treatment::NaiveNonHuman, "zeta-model"),
    ] {
        let db = build(&sessions, treatment);
        assert_eq!(
            metrics(&db)
                .favorite_model(window())
                .unwrap()
                .model
                .as_deref(),
            Some(favorite)
        );
    }
}

#[test]
fn dashboard_and_session_reads_agree_in_one_snapshot() {
    let sessions = [
        (
            "a",
            vec![
                reply("a-agent", 0, "synthetic-model"),
                input("a-automated", 5, "Synthetic dispatched"),
                input("a-human", 10, "Synthetic human"),
                tool("a-call", 11, "synthetic-model"),
                result("a-result", 12),
            ],
            vec!["a-automated"],
        ),
        (
            "b",
            vec![
                input("b-human", 0, "Synthetic human"),
                tool("b-first", 1, "synthetic-model"),
                result("b-first-result", 2),
                input("b-automated", 3, "Synthetic dispatched"),
                tool("b-second", 4, "synthetic-model"),
                result("b-second-result", 5),
            ],
            vec!["b-automated"],
        ),
        (
            "c",
            vec![
                input("c-automated", 0, "Synthetic dispatched"),
                tool("c-call", 1, "synthetic-model"),
                result("c-result", 2),
            ],
            vec!["c-automated"],
        ),
    ];
    let db = build(&sessions, Treatment::Confirmed);
    let m = metrics(&db);
    let ids = ["a", "b", "c"];
    m.read_snapshot(|m| {
        let counts = m.counts(window(), TypingRate::default())?;
        let windows = m.session_windows(window(), &ids)?;
        let per_session: u64 = windows
            .values()
            .map(|w| match w {
                SessionWindow::Indexed { human_messages, .. } => human_messages.unwrap(),
                SessionWindow::Missing => panic!("missing"),
            })
            .sum();
        assert_eq!(counts.human_messages, Some(per_session));
        assert_eq!(per_session, 2);
        let hands_off = m.hands_off(window())?;
        let rows = m.session_hands_off(window(), &ids)?;
        let mut listed = 0;
        for id in ids {
            let SessionHandsOff::Measured { n, .. } = rows[id] else {
                panic!("measured")
            };
            let SessionStretches::Measured { stretches } = m.session_stretches(window(), id)?
            else {
                panic!("measured")
            };
            assert_eq!(n, stretches.len() as u64);
            listed += n;
            for stretch in stretches {
                assert!(!stretch.start_uuid.ends_with("automated"));
                assert!(!stretch.end_uuid.ends_with("automated"));
            }
        }
        assert_eq!(hands_off.n, Some(listed));
        assert_eq!(listed, 2);
        Ok(())
    })
    .unwrap();
}
