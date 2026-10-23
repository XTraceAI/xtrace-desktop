//! M-21 session warnings read from a real store: each rule fires from the
//! stored facts it names, automatic inputs never count as a person's, and a
//! rule whose inputs were not kept says so instead of reporting nothing.
use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use xt_fixtures::TempDb;
use xt_metrics::{
    CheckState, MetricsDb, Missing, RepeatThresholds, SessionRepeats, SessionWarnings, WarningKind,
    Window,
};
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, batch::IngestBatch, retention::RetentionMode,
    tool_sent::ToolSentKind,
};

const SESSION: &str = "s";

fn ms(value: &str) -> i64 {
    value.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn at(minute: u32, second: u32) -> String {
    format!("2026-09-07T12:{minute:02}:{second:02}Z")
}
fn record(value: Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn says(uuid: &str, time: &str, text: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user","timestamp":time,
        "message":{"role":"user","content":[{"type":"text","text":text}]}}))
}
fn reply(uuid: &str, time: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"assistant","timestamp":time,
        "message":{"role":"assistant","content":[{"type":"text","text":"Synthetic reply"}]}}))
}
/// One assistant record calling each `(id, tool, input)`.
fn calls(uuid: &str, time: &str, calls: &[(&str, &str, Value)]) -> CanonicalRecord {
    let content: Vec<Value> = calls
        .iter()
        .map(|(id, name, input)| json!({"type":"tool_use","id":id,"name":name,"input":input}))
        .collect();
    record(json!({"uuid":uuid,"type":"assistant","timestamp":time,
        "message":{"role":"assistant","content":content}}))
}
fn result(uuid: &str, time: &str, call: &str, failed: Option<bool>, text: &str) -> CanonicalRecord {
    let mut block = json!({"type":"tool_result","tool_use_id":call,"content":text});
    if let Some(failed) = failed {
        block["is_error"] = failed.into();
    }
    record(json!({"uuid":uuid,"type":"user","timestamp":time,
        "message":{"role":"user","content":[block]}}))
}

fn seed(db: &mut TempDb, id: &str, host: &str, rows: &[CanonicalRecord], keep_content: bool) {
    let mut meta = SessionMeta::new(id, host, SessionSource::Transcript);
    meta.native_session_id = Some(id.into());
    meta.surface = Some("cli".into());
    db.store_mut().upsert_session(&meta, false).unwrap();
    db.store_mut()
        .set_retention_mode(if keep_content {
            RetentionMode::FullContent
        } else {
            RetentionMode::MetadataOnly
        })
        .unwrap();
    db.store_mut()
        .upsert_records(id, rows, keep_content)
        .unwrap();
}

fn warnings(db: &TempDb, id: &str) -> SessionWarnings {
    MetricsDb::open(db.path())
        .unwrap()
        .session_warnings(window(), RepeatThresholds::default(), &TimeZone::UTC)
        .unwrap()
        .into_iter()
        .find(|s| s.session_id == id)
        .unwrap()
}
fn uuids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn conversation() -> Vec<CanonicalRecord> {
    let pnpm = json!({"command": "pnpm test"});
    let read = json!({"file_path": "/a.rs"});
    let todo = json!({"todos": []});
    vec![
        says("h1", &at(0, 0), "Fix the build please"),
        calls("a1", &at(0, 1), &[("t1", "Bash", pnpm.clone())]),
        result("r1", &at(0, 2), "t1", Some(true), "1 test failed"),
        calls("a2", &at(0, 3), &[("t2", "Bash", pnpm.clone())]),
        result("r2", &at(0, 4), "t2", Some(true), "1 test failed"),
        calls(
            "a3",
            &at(0, 5),
            &[("t3", "Edit", json!({"file_path": "/b.rs"}))],
        ),
        result(
            "r3",
            &at(0, 6),
            "t3",
            Some(true),
            "The user doesn't want to proceed with this tool use. The tool use was rejected.",
        ),
        // Claude Code's marker right after the declined call: one interruption.
        says(
            "i1",
            &at(0, 7),
            "[Request interrupted by user for tool use]",
        ),
        // Two identical automatic inputs: the classification says no person
        // sent them, so they are never a repeated prompt or a correction.
        says(
            "x1",
            &at(0, 8),
            "<system-reminder>stop it now please</system-reminder>",
        ),
        reply("a4", &at(0, 9)),
        says(
            "x2",
            &at(0, 10),
            "<system-reminder>stop it now please</system-reminder>",
        ),
        says("h2", &at(1, 0), "still not working"),
        reply("a5", &at(1, 30)),
        says("h3", &at(2, 0), "fix  the build please"),
        calls(
            "a6",
            &at(2, 30),
            &[
                ("t4", "Read", read.clone()),
                ("t5", "Read", read.clone()),
                ("t6", "Read", read.clone()),
                ("t7", "Read", read.clone()),
                ("t8", "Read", read.clone()),
                ("t9", "Read", read),
            ],
        ),
        reply("a7", &at(6, 30)),
        says("h4", &at(7, 0), "I still need an export button"),
        // Six task-list updates in four active minutes: M-20 calls this
        // stretch circling; they are expected repeats, so M-21 does not.
        calls(
            "a8",
            &at(7, 30),
            &[
                ("u1", "TodoWrite", todo.clone()),
                ("u2", "TodoWrite", todo.clone()),
                ("u3", "TodoWrite", todo.clone()),
                ("u4", "TodoWrite", todo.clone()),
                ("u5", "TodoWrite", todo.clone()),
                ("u6", "TodoWrite", todo),
            ],
        ),
        reply("a9", &at(11, 30)),
    ]
}

#[test]
fn every_rule_fires_from_kept_content() {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, SESSION, "claude", &conversation(), true);
    let found = warnings(&db, SESSION);
    let summary: Vec<_> = found
        .warnings
        .iter()
        .map(|w| (w.kind, w.record_uuids.clone(), w.reason.as_str()))
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                WarningKind::RepeatedFailure,
                uuids(&["r1", "r2"]),
                "the same Bash call failed twice (12:00, 12:00)"
            ),
            (
                WarningKind::Interruption,
                uuids(&["r3", "i1"]),
                "you declined a call to Edit (12:00)"
            ),
            (
                WarningKind::Correction,
                uuids(&["h2"]),
                "you wrote \"still not\" after a reply (12:01)"
            ),
            (
                WarningKind::Circling,
                uuids(&["h3", "a6", "a7"]),
                "went in circles: 5 repeated calls in 4 active minutes, most often the same Read call (6 times) (from 12:02)"
            ),
            (
                WarningKind::RepeatedPrompt,
                uuids(&["h1", "h3"]),
                "same prompt sent twice (12:00, 12:02)"
            ),
        ]
    );
    for kind in WarningKind::ALL {
        assert_eq!(found.checks.get(kind), &CheckState::Complete, "{kind:?}");
    }
    assert_eq!((found.person_messages, found.person_messages_read), (4, 4));
    assert_eq!((found.tool_results, found.tool_results_known), (3, 3));

    // M-20, the Dashboard's number, still compares every call: the
    // task-list stretch M-21 leaves alone is circling there.
    let SessionRepeats::Measured { stretches, .. } = MetricsDb::open(db.path())
        .unwrap()
        .session_repeats(window(), SESSION, RepeatThresholds::default())
        .unwrap()
    else {
        panic!("measured");
    };
    let circling: Vec<_> = stretches
        .iter()
        .filter(|s| s.circling == Some(true))
        .map(|s| s.start_uuid.as_str())
        .collect();
    assert_eq!(circling, vec!["h3", "h4"]);
}

#[test]
fn outcomes_are_read_without_kept_content_and_a_preview_is_read() {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, SESSION, "claude", &conversation(), false);
    // The index keeps a short preview of a person's message whatever the
    // retention setting says; keep one, for the correction only.
    Connection::open(db.path())
        .unwrap()
        .execute(
            "INSERT INTO record_previews(record_uuid,session_id,kind,text,truncated,rule_version)
             VALUES('h2',?1,'person','still not working',0,1)",
            [SESSION],
        )
        .unwrap();
    let found = warnings(&db, SESSION);
    let kinds: Vec<_> = found.warnings.iter().map(|w| w.kind).collect();
    // Outcomes, interruptions and circling are metadata; the correction
    // reads the preview; the other prompts' words were not kept.
    assert_eq!(
        kinds,
        vec![
            WarningKind::RepeatedFailure,
            WarningKind::Interruption,
            WarningKind::Correction,
            WarningKind::Circling
        ]
    );
    assert_eq!(found.checks.repeated_failure, CheckState::Complete);
    assert_eq!(found.checks.interruption, CheckState::Complete);
    assert_eq!(found.checks.circling, CheckState::Complete);
    assert_eq!(
        found.checks.correction,
        CheckState::Incomplete {
            missing: vec![Missing::TextNotKept { messages: 2 }]
        }
    );
    assert_eq!(
        found.checks.repeated_prompt,
        CheckState::Incomplete {
            missing: vec![Missing::TextNotKept { messages: 3 }]
        }
    );
    assert_eq!((found.person_messages, found.person_messages_read), (4, 1));
}

#[test]
fn results_indexed_before_outcomes_were_kept_are_said_not_saved() {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, SESSION, "claude", &conversation(), false);
    // As an index written before migration 27 holds them.
    Connection::open(db.path())
        .unwrap()
        .execute_batch("DELETE FROM tool_results; UPDATE tool_uses SET call_id=NULL;")
        .unwrap();
    let found = warnings(&db, SESSION);
    assert!(
        found
            .warnings
            .iter()
            .all(|w| w.kind != WarningKind::RepeatedFailure)
    );
    // The interruption marker is still there, on its own now.
    let interruptions: Vec<_> = found
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::Interruption)
        .map(|w| w.record_uuids.clone())
        .collect();
    assert_eq!(interruptions, vec![uuids(&["i1"])]);
    let missing = CheckState::Incomplete {
        missing: vec![Missing::OutcomeNotSaved { records: 3 }],
    };
    assert_eq!(found.checks.repeated_failure, missing);
    assert_eq!(found.checks.interruption, missing);
}

#[test]
fn codex_results_without_a_flag_are_unknown() {
    let mut db = TempDb::empty().unwrap();
    let bash = json!({"command": "cargo test"});
    seed(
        &mut db,
        "codex-s",
        "codex",
        &[
            says("h1", &at(0, 0), "make the tests pass"),
            calls("a1", &at(0, 1), &[("c1", "exec_command", bash.clone())]),
            result("r1", &at(0, 2), "c1", None, "Exit code: 1"),
            calls("a2", &at(0, 3), &[("c2", "exec_command", bash)]),
            result("r2", &at(0, 4), "c2", None, "Exit code: 1"),
            reply("a3", &at(0, 5)),
        ],
        false,
    );
    let found = warnings(&db, "codex-s");
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    assert_eq!(
        found.checks.repeated_failure,
        CheckState::Incomplete {
            missing: vec![Missing::OutcomeUnknown { results: 2 }]
        }
    );
    assert_eq!((found.tool_results, found.tool_results_known), (2, 0));
}

/// Read one session the way the writer does, with its tool-sent claims, so
/// previews and proofs are kept as ingestion keeps them.
fn ingest(
    db: &mut TempDb,
    id: &str,
    host: &str,
    rows: &[CanonicalRecord],
    tool_sent: &[Option<ToolSentKind>],
    keep_content: bool,
) {
    let mut meta = SessionMeta::new(id, host, SessionSource::ReadersCli);
    meta.native_session_id = Some(id.into());
    meta.surface = Some("cli".into());
    db.store_mut()
        .set_retention_mode(if keep_content {
            RetentionMode::FullContent
        } else {
            RetentionMode::MetadataOnly
        })
        .unwrap();
    let mut batch = IngestBatch::new(&meta, rows, keep_content);
    batch.tool_sent = tool_sent;
    let saved = db.store_mut().apply_ingest_batch(&batch).unwrap();
    assert!(saved.records.iter().all(|r| r.disposition.is_accepted()));
}

#[test]
fn codex_s_proven_interrupted_turn_note_is_an_interruption() {
    let mut db = TempDb::empty().unwrap();
    let rows = [
        says("h1", &at(0, 0), "make the tests pass"),
        reply("a1", &at(0, 1)),
        says(
            "t1",
            &at(0, 2),
            "<turn_aborted>\nThe user interrupted the previous turn on purpose.\n</turn_aborted>",
        ),
        says("h2", &at(0, 3), "use the other branch instead"),
    ];
    ingest(
        &mut db,
        "codex-s",
        "codex",
        &rows,
        &[None, None, Some(ToolSentKind::CodexTurnAborted), None],
        false,
    );
    let found = warnings(&db, "codex-s");
    let interruptions: Vec<_> = found
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::Interruption)
        .map(|w| (w.record_uuids.clone(), w.reason.as_str()))
        .collect();
    assert_eq!(
        interruptions,
        vec![(uuids(&["t1"]), "you interrupted the agent (12:00)")]
    );
    assert_eq!(found.checks.interruption, CheckState::Complete);
    // The proven note is not the person's message.
    assert_eq!(found.person_messages, 2);
}

#[test]
fn a_decline_and_its_marker_at_one_moment_are_one_interruption_in_source_order() {
    let mut db = TempDb::empty().unwrap();
    let rows = [
        says("h1", &at(0, 0), "edit the parser"),
        calls(
            "a1",
            &at(0, 1),
            &[("t1", "Edit", json!({"file_path": "/p.rs"}))],
        ),
        result(
            "z-decline",
            &at(0, 2),
            "t1",
            Some(true),
            "The user doesn't want to proceed with this tool use. The tool use was rejected.",
        ),
        says(
            "a-marker",
            &at(0, 2),
            "[Request interrupted by user for tool use]",
        ),
    ];
    seed(&mut db, SESSION, "claude", &rows, false);
    // Claude Code writes the marker after the result it follows.
    Connection::open(db.path())
        .unwrap()
        .execute(
            "UPDATE records SET parent_uuid='z-decline' WHERE uuid='a-marker'",
            [],
        )
        .unwrap();
    let found = warnings(&db, SESSION);
    let interruptions: Vec<_> = found
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::Interruption)
        .map(|w| (w.record_uuids.clone(), w.reason.as_str()))
        .collect();
    assert_eq!(
        interruptions,
        vec![(
            uuids(&["z-decline", "a-marker"]),
            "you declined a call to Edit (12:00)"
        )]
    );
}

#[test]
fn an_interrupted_call_and_a_permission_denial_are_not_failures() {
    let mut db = TempDb::empty().unwrap();
    let bash = json!({"command": "cargo test"});
    let rows = [
        says("h1", &at(0, 0), "run the tests please"),
        calls("a1", &at(0, 1), &[("t1", "Bash", bash.clone())]),
        result(
            "r1",
            &at(0, 2),
            "t1",
            Some(true),
            "[Request interrupted by user for tool use]",
        ),
        reply("a2", &at(0, 3)),
        calls("a3", &at(0, 4), &[("t2", "Bash", bash.clone())]),
        result(
            "r2",
            &at(0, 5),
            "t2",
            Some(true),
            "Permission for this action was denied. Reason: outside the project.",
        ),
        calls("a4", &at(0, 6), &[("t3", "Bash", bash)]),
        result(
            "r3",
            &at(0, 7),
            "t3",
            Some(true),
            "Permission to use Bash has been denied because Claude Code is running in don't ask mode.",
        ),
        reply("a5", &at(0, 8)),
    ];
    seed(&mut db, SESSION, "claude", &rows, false);
    let found = warnings(&db, SESSION);
    let summary: Vec<_> = found
        .warnings
        .iter()
        .map(|w| (w.kind, w.record_uuids.clone(), w.reason.as_str()))
        .collect();
    // The interrupted call is the person's interruption; the two denials by
    // the host's own check are neither a decline nor a repeated failure.
    assert_eq!(
        summary,
        vec![(
            WarningKind::Interruption,
            uuids(&["r1"]),
            "you interrupted a call to Bash (12:00)"
        )]
    );
    assert_eq!(found.checks.repeated_failure, CheckState::Complete);
    assert_eq!(found.checks.interruption, CheckState::Complete);
    assert_eq!((found.tool_results, found.tool_results_known), (3, 3));
}

/// Two prompts identical for their first 280 characters and different after,
/// and a reply's correction that comes after character 280.
fn long_messages() -> Vec<CanonicalRecord> {
    let opening = "please look at the module layout and the tests ".repeat(7);
    assert!(opening.chars().count() > 300);
    vec![
        says("h1", &at(0, 0), &format!("{opening} then fix the parser")),
        reply("a1", &at(0, 1)),
        says("h2", &at(0, 2), &format!("{opening} that is wrong")),
        reply("a2", &at(0, 3)),
        says("h3", &at(0, 4), &format!("{opening} then fix the lexer")),
        reply("a3", &at(0, 5)),
    ]
}

#[test]
fn whole_kept_messages_are_read_past_the_preview_length() {
    let mut db = TempDb::empty().unwrap();
    ingest(&mut db, SESSION, "claude", &long_messages(), &[], true);
    let found = warnings(&db, SESSION);
    let summary: Vec<_> = found
        .warnings
        .iter()
        .map(|w| (w.kind, w.record_uuids.clone()))
        .collect();
    // The correction after character 280 is found, and the two prompts that
    // differ only after it are not the same prompt.
    assert_eq!(summary, vec![(WarningKind::Correction, uuids(&["h2"]))]);
    assert_eq!(found.checks.correction, CheckState::Complete);
    assert_eq!(found.checks.repeated_prompt, CheckState::Complete);
    assert_eq!((found.person_messages, found.person_messages_read), (3, 3));
}

#[test]
fn a_cut_preview_never_reads_as_the_whole_message() {
    let mut db = TempDb::empty().unwrap();
    ingest(&mut db, SESSION, "claude", &long_messages(), &[], false);
    let found = warnings(&db, SESSION);
    // Only the previews' first 280 characters were kept: no correction is
    // found there, the prompts look the same there, and neither rule says
    // it read everything.
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    // The two messages sent after a reply.
    assert_eq!(
        found.checks.correction,
        CheckState::Incomplete {
            missing: vec![Missing::TextCut { messages: 2 }]
        }
    );
    assert_eq!(
        found.checks.repeated_prompt,
        CheckState::Incomplete {
            missing: vec![Missing::TextCut { messages: 3 }]
        }
    );
    assert_eq!((found.person_messages, found.person_messages_read), (3, 0));
}
