//! One span's detail: the most-used tool, the output tokens and the last
//! message a person typed, each restricted to the span and each taken from the
//! rule that already owns it. Every expectation is an exact value from the
//! synthetic records below.
use jiff::Timestamp;
use serde_json::{Value, json};
use xt_fixtures::TempDb;
use xt_metrics::{MetricsDb, PromptText, SessionWindow, SpanDetail, SpanPrompt, SpanTool, Window};
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource,
    confirmation::{AutomatedInputProof, ConfirmationDisposition, EvidenceKind},
    human_input::InputAdjustment,
    retention::RetentionMode,
};

fn ms(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_millisecond()
}
fn at(hour: u32, minute: u32) -> String {
    format!("2026-09-07T{hour:02}:{minute:02}:00Z")
}
fn record(value: Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn input(uuid: &str, time: &str, text: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user","timestamp":time,
        "message":{"role":"user","content":[{"type":"text","text":text}]}}))
}
fn sidechain(uuid: &str, time: &str, text: &str) -> CanonicalRecord {
    record(
        json!({"uuid":uuid,"type":"user","timestamp":time,"isSidechain":true,
        "message":{"role":"user","content":[{"type":"text","text":text}]}}),
    )
}
/// One assistant record calling `tools`, as its own response.
fn calls(uuid: &str, time: &str, tools: &[&str], output: u64) -> CanonicalRecord {
    let content: Vec<Value> = tools
        .iter()
        .map(|name| json!({"type":"tool_use","name":name,"input":{"path":"synthetic"}}))
        .collect();
    record(
        json!({"uuid":uuid,"type":"assistant","timestamp":time,"requestId":format!("req-{uuid}"),
        "message":{"id":format!("msg-{uuid}"),"role":"assistant","model":"fixture-model-v1",
            "content":content,
            "usage":{"input_tokens":10,"output_tokens":output,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
    )
}
/// One record of a response that is repeated across records, as Claude
/// writes one record per content block with the response's usage on each.
fn part(uuid: &str, time: &str, response: &str, output: u64) -> CanonicalRecord {
    record(
        json!({"uuid":uuid,"type":"assistant","timestamp":time,"requestId":format!("req-{response}"),
        "message":{"id":format!("msg-{response}"),"role":"assistant","model":"fixture-model-v1",
            "content":[{"type":"text","text":"Synthetic reply"}],
            "usage":{"input_tokens":10,"output_tokens":output,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
    )
}

fn seed(db: &mut TempDb, session: &str, rows: &[CanonicalRecord], keep_content: bool) {
    let mut meta = SessionMeta::new(session, "claude", SessionSource::Transcript);
    meta.native_session_id = Some(session.into());
    meta.surface = Some("cli".into());
    db.store_mut().upsert_session(&meta, false).unwrap();
    // The saved policy restricts the writer further, and defaults to
    // metadata only; content is kept only where both allow it.
    db.store_mut()
        .set_retention_mode(if keep_content {
            RetentionMode::FullContent
        } else {
            RetentionMode::MetadataOnly
        })
        .unwrap();
    db.store_mut()
        .upsert_records(session, rows, keep_content)
        .unwrap();
}

fn confirm(db: &mut TempDb, session: &str, uuid: &str) {
    let report = db
        .store_mut()
        .apply_automated_input_confirmations(
            &[AutomatedInputProof {
                record_uuid: uuid.into(),
                session_id: session.into(),
                native_session_id: session.into(),
                parent_host: Host::Codex,
                parent_session_id: "codex-synthetic-parent".into(),
                parent_tool_call_id: format!("call-{session}"),
                parent_operation_index: 0,
                parent_result_id: None,
                evidence_kind: EvidenceKind::AgentDispatch,
                matcher_version: 1,
            }],
            1,
        )
        .unwrap();
    assert_eq!(report.dispositions, [ConfirmationDisposition::Confirmed]);
}

fn indexed(detail: SpanDetail) -> (SpanTool, Option<u64>, SpanPrompt) {
    match detail {
        SpanDetail::Indexed {
            tool,
            output_tokens,
            prompt,
            ..
        } => (tool, output_tokens, prompt),
        SpanDetail::Missing => panic!("the seeded session is indexed"),
    }
}

fn stored(text: &str) -> PromptText {
    PromptText::Stored {
        text: text.into(),
        truncated: false,
    }
}

/// Three spans of one session, each separated by more than twenty minutes:
///
/// * 09:00–09:10 — a prompt, then Bash five times.
/// * 10:00–10:12 — a prompt, Read twice and Bash once, and one response
///   repeated over two records whose usage is restated (5 then 40 output).
/// * 11:00–11:05 — no prompt of its own; Grep once.
fn three_spans() -> TempDb {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            input("p1", &at(9, 0), "first   task\nplease"),
            calls("a1", &at(9, 5), &["Bash", "Bash", "Bash"], 7),
            calls("a2", &at(9, 10), &["Bash", "Bash"], 3),
            input("p2", &at(10, 0), "make the PR link exact, not inferred"),
            calls("a3", &at(10, 2), &["Read", "Bash"], 11),
            calls("a4", &at(10, 4), &["Read"], 13),
            part("a5", &at(10, 10), "r", 5),
            part("a6", &at(10, 12), "r", 40),
            calls("a7", &at(11, 0), &["Grep"], 2),
            part("a8", &at(11, 5), "t", 1),
        ],
        true,
    );
    db
}

#[test]
fn the_span_reports_its_own_most_used_tool() {
    let db = three_spans();
    let metrics = MetricsDb::open(db.path()).unwrap();
    // The session as a whole calls Bash six times, but this span calls Read
    // twice and Bash once.
    let (tool, _, _) = indexed(
        metrics
            .span_detail("s", ms(&at(10, 0)), ms(&at(10, 12)))
            .unwrap(),
    );
    assert_eq!(
        tool,
        SpanTool::Called {
            name: "Read".into(),
            calls: 2
        }
    );
    let (tool, _, _) = indexed(
        metrics
            .span_detail("s", ms(&at(9, 0)), ms(&at(9, 10)))
            .unwrap(),
    );
    assert_eq!(
        tool,
        SpanTool::Called {
            name: "Bash".into(),
            calls: 5
        }
    );
}

#[test]
fn a_tie_resolves_to_the_same_tool_every_time() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "tie",
        &[calls("t1", &at(9, 0), &["Write", "Edit"], 1)],
        false,
    );
    let metrics = MetricsDb::open(db.path()).unwrap();
    let (tool, _, _) = indexed(
        metrics
            .span_detail("tie", ms(&at(9, 0)), ms(&at(9, 0)))
            .unwrap(),
    );
    assert_eq!(
        tool,
        SpanTool::Called {
            name: "Edit".into(),
            calls: 1
        }
    );
}

#[test]
fn output_is_the_shared_selection_over_the_span() {
    let db = three_spans();
    let metrics = MetricsDb::open(db.path()).unwrap();
    // 11 + 13 from the two single-record responses, and 40 — not 45 — from
    // the response restated over two records.
    let (start, end) = (ms(&at(10, 0)), ms(&at(10, 12)));
    let (_, output, _) = indexed(metrics.span_detail("s", start, end).unwrap());
    assert_eq!(output, Some(64));
    // The very slice session_windows takes over the span's interval.
    let SessionWindow::Indexed { tokens, .. } = &metrics
        .session_windows(Window::new(start, end + 1).unwrap(), &["s"])
        .unwrap()["s"]
    else {
        panic!("indexed");
    };
    assert_eq!(output, tokens.counters.output_tokens);
    // A single-event span includes its one event.
    let (_, output, _) = indexed(
        metrics
            .span_detail("s", ms(&at(11, 5)), ms(&at(11, 5)))
            .unwrap(),
    );
    assert_eq!(output, Some(1));
}

#[test]
fn the_prompt_is_the_spans_own_else_the_latest_before_it() {
    let db = three_spans();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let (_, _, prompt) = indexed(
        metrics
            .span_detail("s", ms(&at(10, 0)), ms(&at(10, 12)))
            .unwrap(),
    );
    assert_eq!(
        prompt,
        SpanPrompt::Found {
            at_ms: ms(&at(10, 0)),
            in_span: true,
            record_uuid: "p2".into(),
            text: stored("make the PR link exact, not inferred"),
        }
    );
    // The last span typed nothing: its prompt is the one before it started.
    let (_, _, prompt) = indexed(
        metrics
            .span_detail("s", ms(&at(11, 0)), ms(&at(11, 5)))
            .unwrap(),
    );
    assert_eq!(
        prompt,
        SpanPrompt::Found {
            at_ms: ms(&at(10, 0)),
            in_span: false,
            record_uuid: "p2".into(),
            text: stored("make the PR link exact, not inferred"),
        }
    );
    // Whitespace runs collapse to one line.
    let (_, _, prompt) = indexed(
        metrics
            .span_detail("s", ms(&at(9, 0)), ms(&at(9, 10)))
            .unwrap(),
    );
    assert_eq!(
        prompt,
        SpanPrompt::Found {
            at_ms: ms(&at(9, 0)),
            in_span: true,
            record_uuid: "p1".into(),
            text: stored("first task please"),
        }
    );
}

#[test]
fn sidechain_and_confirmed_automated_inputs_are_never_the_prompt() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "auto",
        &[
            input("human", &at(9, 0), "a person typed this"),
            calls("c1", &at(9, 1), &["Read"], 1),
            sidechain("side", &at(9, 2), "a subagent's own input"),
            input("dispatch", &at(9, 3), "an agent dispatched this"),
            calls("c2", &at(9, 4), &["Read"], 1),
        ],
        true,
    );
    // Unconfirmed, the dispatched input is still classified as a person's.
    let metrics = MetricsDb::open(db.path()).unwrap();
    let (_, _, prompt) = indexed(
        metrics
            .span_detail("auto", ms(&at(9, 0)), ms(&at(9, 4)))
            .unwrap(),
    );
    assert!(matches!(prompt, SpanPrompt::Found { at_ms, .. } if at_ms == ms(&at(9, 3))));
    drop(metrics);
    confirm(&mut db, "auto", "dispatch");
    let metrics = MetricsDb::open(db.path()).unwrap();
    let (_, _, prompt) = indexed(
        metrics
            .span_detail("auto", ms(&at(9, 0)), ms(&at(9, 4)))
            .unwrap(),
    );
    assert_eq!(
        prompt,
        SpanPrompt::Found {
            at_ms: ms(&at(9, 0)),
            in_span: true,
            record_uuid: "human".into(),
            text: stored("a person typed this"),
        }
    );
}

#[test]
fn a_session_with_only_delegated_inputs_has_no_prompt() {
    // A session that only ever received tool results and a sidechain input
    // has no person's message at all.
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "quiet",
        &[
            sidechain("side", &at(9, 0), "delegated"),
            calls("c", &at(9, 1), &["Read"], 1),
        ],
        true,
    );
    let metrics = MetricsDb::open(db.path()).unwrap();
    let (_, _, prompt) = indexed(
        metrics
            .span_detail("quiet", ms(&at(9, 0)), ms(&at(9, 1)))
            .unwrap(),
    );
    assert_eq!(prompt, SpanPrompt::NoMessage);
}

#[test]
fn unknowns_stay_unknown() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "bare",
        &[
            // A person's message whose words metadata-only retention dropped.
            input("typed", &at(9, 0), "words not kept"),
            // An assistant record with no usage and no content: neither its
            // output nor its tool calls are known.
            record(
                json!({"uuid":"silent","type":"assistant","timestamp":at(9, 1),
                "message":{"role":"assistant"}}),
            ),
        ],
        false,
    );
    let metrics = MetricsDb::open(db.path()).unwrap();
    let (tool, output, prompt) = indexed(
        metrics
            .span_detail("bare", ms(&at(9, 0)), ms(&at(9, 1)))
            .unwrap(),
    );
    assert_eq!(tool, SpanTool::Unknown);
    assert_eq!(output, None);
    assert_eq!(
        prompt,
        SpanPrompt::Found {
            at_ms: ms(&at(9, 0)),
            in_span: true,
            record_uuid: "typed".into(),
            text: PromptText::NotStored,
        }
    );
    drop(metrics);

    // A user record with no content cannot be classified: when it is the
    // newest candidate, which message a person typed last is unknown.
    seed(
        &mut db,
        "unclassified",
        &[
            input("known", &at(9, 0), "classified"),
            record(json!({"uuid":"blank","type":"user","timestamp":at(9, 2),
                "message":{"role":"user"}})),
        ],
        true,
    );
    let metrics = MetricsDb::open(db.path()).unwrap();
    let (_, _, prompt) = indexed(
        metrics
            .span_detail("unclassified", ms(&at(9, 0)), ms(&at(9, 2)))
            .unwrap(),
    );
    assert_eq!(prompt, SpanPrompt::Unclassified);
}

#[test]
fn text_only_spans_call_no_tool_and_unknown_sessions_are_missing() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "talk",
        &[
            input("q", &at(9, 0), "just a question"),
            part("r", &at(9, 1), "answer", 4),
        ],
        true,
    );
    let metrics = MetricsDb::open(db.path()).unwrap();
    let (tool, output, _) = indexed(
        metrics
            .span_detail("talk", ms(&at(9, 0)), ms(&at(9, 1)))
            .unwrap(),
    );
    assert_eq!(tool, SpanTool::NoCalls);
    assert_eq!(output, Some(4));
    assert_eq!(
        metrics.span_detail("nobody", 0, 1).unwrap(),
        SpanDetail::Missing
    );
    assert!(metrics.span_detail("talk", 2, 1).is_err());
}

#[test]
fn a_wrapped_input_withholds_its_words_rather_than_showing_the_wrapper() {
    let mut db = TempDb::empty().unwrap();
    let mut meta = SessionMeta::new("wrapped", "codex", SessionSource::Transcript);
    meta.native_session_id = Some("wrapped".into());
    db.store_mut().upsert_session(&meta, false).unwrap();
    db.store_mut()
        .set_retention_mode(RetentionMode::FullContent)
        .unwrap();
    let wrapper = r#"<send_user_message_question_reply>[{"answer":"yes"}]</send_user_message_question_reply>"#;
    db.store_mut()
        .upsert_records(
            "wrapped",
            &[
                input("reply", &at(9, 0), wrapper),
                calls("c", &at(9, 1), &["Read"], 1),
            ],
            true,
        )
        .unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    // Before any adjustment the whole record is the person's.
    let (_, _, prompt) = indexed(
        metrics
            .span_detail("wrapped", ms(&at(9, 0)), ms(&at(9, 1)))
            .unwrap(),
    );
    assert!(matches!(
        prompt,
        SpanPrompt::Found {
            text: PromptText::Stored { .. },
            ..
        }
    ));
    drop(metrics);
    // The accepted adjustment says only the three answer characters are.
    let report = db
        .store_mut()
        .apply_human_input_adjustments(&[InputAdjustment {
            record_uuid: "reply".into(),
            session_id: "wrapped".into(),
            original_ts: at(9, 0),
            original_length: i64::try_from(wrapper.chars().count()).unwrap(),
            retained_length: Some(3),
            reason: "question_reply".into(),
            native_item_id: None,
        }])
        .unwrap();
    assert_eq!(report.applied, 1);
    let metrics = MetricsDb::open(db.path()).unwrap();
    let (_, _, prompt) = indexed(
        metrics
            .span_detail("wrapped", ms(&at(9, 0)), ms(&at(9, 1)))
            .unwrap(),
    );
    assert_eq!(
        prompt,
        SpanPrompt::Found {
            at_ms: ms(&at(9, 0)),
            in_span: true,
            record_uuid: "reply".into(),
            text: PromptText::Wrapped,
        }
    );
}
