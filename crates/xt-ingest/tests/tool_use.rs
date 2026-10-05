//! ING-05: structural tool, MCP, skill/subagent, slash-command and hook-summary
//! extraction, and the replay identity that keeps one native event one row.
//!
//! The kinds are the stable strings documented in `xt_store::tool_use`. Slash
//! commands and hook summaries are structural events, not assistant tool calls:
//! they never enter a record's `tool_use_count` or its tool-call rows. No test
//! here asserts that a shell command, an argument or an output was retained,
//! because none of them ever is.

use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_ingest::{
    canonical::{Parsed, ParsedRecord, SourceContext, StopHookSummary, parse_line},
    tool_use::{HOOK_EVENT_NAME, classify},
    writer::{WriteBatch, write_batch},
};
use xt_store::{
    SessionSource, Store,
    ingest::{CaptureReceipt, DiscoveredSession, ToolEvent, ToolKind},
};

const SESSION: &str = "00000000-0000-4000-8000-0000000000a1";
const OTHER_SESSION: &str = "00000000-0000-4000-8000-0000000000a2";
const COMMAND_UUID: &str = "00000000-0000-4000-8000-0000000000c1";
const SUMMARY_UUID: &str = "00000000-0000-4000-8000-0000000000h1";

fn parsed(value: Value) -> ParsedRecord {
    match parse_line(&value.to_string()).unwrap() {
        Parsed::Record(record) => *record,
        other => panic!("expected a canonical record, got {other:?}"),
    }
}

fn summary(value: Value) -> StopHookSummary {
    match parse_line(&value.to_string()).unwrap() {
        Parsed::StructuralEvent(summary) => *summary,
        other => panic!("expected a structural summary, got {other:?}"),
    }
}

/// One assistant record carrying every tool shape the mapping must classify.
fn tool_call_record(uuid: &str) -> ParsedRecord {
    parsed(
        json!({"uuid":uuid,"type":"assistant","timestamp":"2026-09-07T12:00:01Z",
        "message":{"role":"assistant","content":[
            {"type":"tool_use","name":"Bash","input":{"command":"gh pr create --title x"}},
            {"type":"tool_use","name":"Edit","input":{"file_path":"/private/synthetic.rs"}},
            {"type":"tool_use","name":"mcp__Claude_Browser__computer","input":{"action":"screenshot"}},
            {"type":"tool_use","name":"Skill","input":{"skill":"memhub:login","args":"--status"}},
            {"type":"tool_use","name":"Agent","input":{"prompt":"synthetic"}},
            {"type":"tool_use","name":"Task","input":{"prompt":"synthetic"}}
        ]}}),
    )
}

/// The user record a slash command is stated by. The command's arguments sit in
/// the same message and must never reach storage or the structural row.
fn command_record(uuid: &str) -> ParsedRecord {
    parsed(
        json!({"uuid":uuid,"type":"user","timestamp":"2026-09-07T12:00:00Z",
        "message":{"role":"user","content":[{"type":"text",
            "text":"<command-name>/memhub:login</command-name>\n<command-args>--status secret-token</command-args>"}]}}),
    )
}

fn hook_summary(uuid: &str) -> StopHookSummary {
    summary(
        json!({"type":"system","subtype":"stop_hook_summary","uuid":uuid,
        "timestamp":"2026-09-07T12:00:02Z","hookCount":3,"preventedContinuation":false,
        "hookInfos":[{"command":"synthetic-private-command","durationMs":5}],
        "hasOutput":true,"totalDurationMs":15}),
    )
}

fn context(session: &str, source: SessionSource) -> SourceContext {
    SourceContext {
        conversation_id: Some(session.into()),
        native_session_id: Some(session.into()),
        source_platform: Some("claude".into()),
        source_surface: Some("cli".into()),
        source: Some(source),
        ..Default::default()
    }
}

fn request<'a>(
    context: &'a SourceContext,
    records: &'a [ParsedRecord],
    hook_summaries: &'a [StopHookSummary],
    receipt: Option<&'a CaptureReceipt>,
    keep_content: bool,
) -> WriteBatch<'a> {
    WriteBatch {
        context,
        declared_host: None,
        records,
        hook_summaries,
        pr_witnesses: &[],
        title: None,
        cwd: None,
        git_branch: None,
        namespace: None,
        keep_content,
        observed_at: 100,
        receipt,
        cursor: None,
        discovery: None,
        checkpoint: None,
    }
}

/// The discovered identity a native Claude scan supplies beside its batch; it
/// is what lets one work record belong to a second session file.
fn discovered(session: &str) -> DiscoveredSession {
    DiscoveredSession {
        host: xt_store::Host::Claude,
        native_session_id: session.into(),
        conversation_id: Some(session.into()),
        surface: Some("cli".into()),
        started_at_ms: None,
        last_observed_at: 100,
        discovery_complete: true,
    }
}

fn receipt(id: &str, session: &str) -> CaptureReceipt {
    CaptureReceipt {
        receipt_id: id.into(),
        session_id: session.into(),
        surface: Some("cli".into()),
        received_at: 100,
    }
}

fn fixture(id: &str) -> Fixture {
    Fixture::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(id),
    )
    .unwrap()
}

/// Name, kind and the three structural details, in stored block order.
type Classified = (
    String,
    Option<ToolKind>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn kinds(store: &Store, session: &str) -> Vec<Classified> {
    store
        .records(session)
        .unwrap()
        .iter()
        .flat_map(|record| record.tool_uses.clone())
        .map(|tool| (tool.name, tool.kind, tool.server, tool.tool, tool.skill))
        .collect()
}

fn scalar(path: &std::path::Path, sql: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}

#[test]
fn structural_kind_mapping_covers_every_tool_shape() {
    for keep_content in [false, true] {
        let mut db = TempDb::empty().unwrap();
        if keep_content {
            db.store_mut()
                .set_retention_mode(xt_store::retention::RetentionMode::FullContent)
                .unwrap();
        }
        let context = context(SESSION, SessionSource::Transcript);
        let records = [tool_call_record("00000000-0000-4000-8000-0000000000b1")];
        write_batch(
            db.store_mut(),
            &request(&context, &records, &[], None, keep_content),
        )
        .unwrap();
        assert_eq!(
            kinds(db.store(), SESSION),
            [
                ("Bash".into(), Some(ToolKind::Builtin), None, None, None),
                ("Edit".into(), Some(ToolKind::Builtin), None, None, None),
                (
                    "mcp__Claude_Browser__computer".into(),
                    Some(ToolKind::Mcp),
                    Some("Claude_Browser".into()),
                    Some("computer".into()),
                    None
                ),
                (
                    "Skill".into(),
                    Some(ToolKind::Skill),
                    None,
                    None,
                    Some("memhub:login".into())
                ),
                ("Agent".into(), Some(ToolKind::Subagent), None, None, None),
                ("Task".into(), Some(ToolKind::Subagent), None, None, None),
            ],
            "keep_content={keep_content}"
        );
    }
}

#[test]
fn malformed_detail_stays_unknown_and_shell_text_is_never_parsed() {
    // Classification reads the name, and for Skill one explicit input field.
    // Nothing else in the input is inspected, whatever it happens to contain.
    let shell = json!({"command": "gh pr create --title secret", "skill": "not-a-skill-tool"});
    assert_eq!(classify("Bash", Some(&shell)).kind, ToolKind::Builtin);
    assert_eq!(
        classify("exec_command", Some(&shell)).kind,
        ToolKind::Builtin
    );
    for name in ["mcp__", "mcp__server", "mcp____tool"] {
        let structure = classify(name, None);
        assert_eq!(structure.kind, ToolKind::Mcp, "{name}");
        assert_eq!((structure.server, structure.tool), (None, None), "{name}");
    }
    assert_eq!(classify("Skill", None).skill, None);
}

#[test]
fn slash_command_and_hook_summary_are_structural_events_that_do_not_inflate_tool_calls() {
    let mut db = TempDb::empty().unwrap();
    let context = context(SESSION, SessionSource::Transcript);
    let records = [command_record(COMMAND_UUID)];
    let summaries = [hook_summary(SUMMARY_UUID)];
    write_batch(
        db.store_mut(),
        &request(&context, &records, &summaries, None, false),
    )
    .unwrap();
    let events = db.store().tool_events(SESSION).unwrap();
    assert_eq!(
        events,
        [
            ToolEvent {
                session_id: SESSION.into(),
                source: SessionSource::Transcript,
                source_event_id: COMMAND_UUID.into(),
                timestamp: Some("2026-09-07T12:00:00Z".into()),
                name: "/memhub:login".into(),
                kind: ToolKind::Command,
                server: None,
                tool: None,
                skill: None,
            },
            ToolEvent {
                session_id: SESSION.into(),
                source: SessionSource::Transcript,
                source_event_id: SUMMARY_UUID.into(),
                timestamp: Some("2026-09-07T12:00:02Z".into()),
                name: HOOK_EVENT_NAME.into(),
                kind: ToolKind::Hook,
                server: None,
                tool: None,
                skill: None,
            },
        ]
    );
    // The command record is an ordinary record with no tool call, and the
    // summary is no record at all.
    let rows = db.store().records(SESSION).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tool_use_count, Some(0));
    assert!(rows[0].tool_uses.is_empty());
    // Neither the command arguments nor the hook's command text is stored.
    assert_eq!(
        scalar(
            db.path(),
            "SELECT count(*) FROM tool_uses WHERE input_json IS NOT NULL"
        ),
        0
    );
    for text in ["--status", "secret-token", "synthetic-private-command"] {
        assert_eq!(
            Connection::open(db.path())
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM tool_uses WHERE name LIKE '%'||?1||'%'",
                    [text],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0,
            "{text}"
        );
    }
}

#[test]
fn one_summary_is_one_hook_event_however_many_hooks_it_reports() {
    let mut db = TempDb::empty().unwrap();
    let context = context(SESSION, SessionSource::Transcript);
    let records = [command_record(COMMAND_UUID)];
    // Two summaries, reporting three and one hook invocation.
    let mut second = hook_summary("00000000-0000-4000-8000-0000000000h2");
    second.hook_count = Some(1);
    let summaries = [hook_summary(SUMMARY_UUID), second];
    write_batch(
        db.store_mut(),
        &request(&context, &records, &summaries, None, false),
    )
    .unwrap();
    let hooks = db
        .store()
        .tool_events(SESSION)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == ToolKind::Hook)
        .collect::<Vec<_>>();
    assert_eq!(hooks.len(), 2, "one event per summary, not per invocation");
    assert!(hooks.iter().all(|event| event.name == HOOK_EVENT_NAME));
}

#[test]
fn duplicate_replay_and_reversed_source_arrival_keep_one_event() {
    let mut outcomes = Vec::new();
    for plugin_first in [false, true] {
        let mut db = TempDb::empty().unwrap();
        let sources = if plugin_first {
            [SessionSource::Plugin, SessionSource::Transcript]
        } else {
            [SessionSource::Transcript, SessionSource::Plugin]
        };
        for (attempt, source) in sources.into_iter().chain(sources).enumerate() {
            let context = context(SESSION, source);
            let records = [command_record(COMMAND_UUID)];
            let summaries = [hook_summary(SUMMARY_UUID)];
            let parent = receipt(&format!("capture-{attempt}"), SESSION);
            write_batch(
                db.store_mut(),
                &request(
                    &context,
                    &records,
                    &summaries,
                    (source == SessionSource::Plugin).then_some(&parent),
                    false,
                ),
            )
            .unwrap();
        }
        let events = db.store().tool_events(SESSION).unwrap();
        assert_eq!(events.len(), 2, "plugin_first={plugin_first}");
        assert_eq!(
            scalar(db.path(), "SELECT count(*) FROM tool_uses"),
            2,
            "no duplicate structural row survives replay"
        );
        // The row keeps whichever source first delivered it; the event itself
        // is the same fact either way.
        outcomes.push(
            events
                .into_iter()
                .map(|event| (event.source_event_id, event.kind, event.name))
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

/// A Claude fork keeps the same immutable lines in two session files. Those are
/// the same two events, not four: each is stored once, under the session that
/// owns its identity, exactly as the copied work record is one record with an
/// additional session context.
#[test]
fn copied_native_contexts_count_one_structural_event_each() {
    for forward in [true, false] {
        let mut db = TempDb::empty().unwrap();
        // The scan order changes which file is read first; ownership does not
        // follow arrival beyond the identity's first canonical home.
        let order = if forward {
            [SESSION, OTHER_SESSION]
        } else {
            [OTHER_SESSION, SESSION]
        };
        for session in order {
            let context = context(session, SessionSource::Transcript);
            let records = [command_record(COMMAND_UUID)];
            let summaries = [hook_summary(SUMMARY_UUID)];
            let discovery = discovered(session);
            let mut batch = request(&context, &records, &summaries, None, false);
            batch.discovery = Some(&discovery);
            write_batch(db.store_mut(), &batch).unwrap();
        }
        // One command event and one hook event in total, both owned by the
        // session that owns the work record, and none duplicated per context.
        assert_eq!(
            scalar(db.path(), "SELECT count(*) FROM tool_uses"),
            2,
            "forward={forward}"
        );
        assert_eq!(scalar(db.path(), "SELECT count(*) FROM records"), 1);
        let owner = order[0];
        let events = db.store().tool_events(owner).unwrap();
        assert_eq!(events.len(), 2, "forward={forward}");
        assert_eq!(
            events
                .iter()
                .map(|event| (event.source_event_id.as_str(), event.kind))
                .collect::<Vec<_>>(),
            [
                (COMMAND_UUID, ToolKind::Command),
                (SUMMARY_UUID, ToolKind::Hook)
            ]
        );
        // The command event sits where the canonical record sits, not where the
        // copy was read.
        let record_owner: String = Connection::open(db.path())
            .unwrap()
            .query_row(
                "SELECT session_id FROM records WHERE uuid=?1",
                [COMMAND_UUID],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(record_owner, owner);
        assert!(
            db.store().tool_events(order[1]).unwrap().is_empty(),
            "the copied context holds no second event"
        );
    }
}

/// Without that native copy relationship the second session does not own the
/// record at all, so it is not attributed the command the record states either.
#[test]
fn a_record_this_session_rejected_contributes_no_structural_event_here() {
    let mut db = TempDb::empty().unwrap();
    let records = [command_record(COMMAND_UUID)];
    for session in [SESSION, OTHER_SESSION] {
        let context = context(session, SessionSource::Transcript);
        write_batch(
            db.store_mut(),
            &request(&context, &records, &[], None, false),
        )
        .unwrap();
    }
    assert_eq!(db.store().tool_events(SESSION).unwrap().len(), 1);
    assert!(
        db.store().tool_events(OTHER_SESSION).unwrap().is_empty(),
        "the rejected record's command belongs to its owning session only"
    );
}

#[test]
fn missing_identity_and_conflicting_facts_roll_the_whole_batch_back() {
    let mut db = TempDb::empty().unwrap();
    let context = context(SESSION, SessionSource::Transcript);
    let records = [command_record(COMMAND_UUID)];

    // A summary with no UUID is unsupported; it is never given an arrival ID.
    let mut anonymous = hook_summary(SUMMARY_UUID);
    anonymous.uuid = None;
    assert!(
        write_batch(
            db.store_mut(),
            &request(&context, &records, &[anonymous], None, false),
        )
        .is_err()
    );
    assert_eq!(scalar(db.path(), "SELECT count(*) FROM records"), 0);
    assert_eq!(scalar(db.path(), "SELECT count(*) FROM tool_uses"), 0);

    write_batch(
        db.store_mut(),
        &request(
            &context,
            &records,
            &[hook_summary(SUMMARY_UUID)],
            None,
            false,
        ),
    )
    .unwrap();
    let before = db.store().tool_events(SESSION).unwrap();
    assert_eq!(before.len(), 2);

    // The same identity replayed with a different command name is a conflict:
    // it fails, and the record that arrived with it does not commit either.
    let conflicting = parsed(
        json!({"uuid":COMMAND_UUID,"type":"user","timestamp":"2026-09-07T12:00:00Z",
        "message":{"role":"user","content":[{"type":"text","text":"<command-name>/other</command-name>"}]}}),
    );
    let fresh = parsed(
        json!({"uuid":"00000000-0000-4000-8000-0000000000b9","type":"assistant",
        "timestamp":"2026-09-07T12:00:05Z","message":{"role":"assistant","content":[]}}),
    );
    assert!(
        write_batch(
            db.store_mut(),
            &request(&context, &[conflicting, fresh], &[], None, false),
        )
        .is_err()
    );
    assert_eq!(db.store().tool_events(SESSION).unwrap(), before);
    assert_eq!(scalar(db.path(), "SELECT count(*) FROM records"), 1);
}

#[test]
fn metadata_and_content_modes_agree_on_structure_and_only_differ_in_retained_input() {
    let mut structures = Vec::new();
    for keep_content in [false, true] {
        let mut db = TempDb::empty().unwrap();
        if keep_content {
            db.store_mut()
                .set_retention_mode(xt_store::retention::RetentionMode::FullContent)
                .unwrap();
        }
        let context = context(SESSION, SessionSource::Transcript);
        let records = [
            command_record(COMMAND_UUID),
            tool_call_record("00000000-0000-4000-8000-0000000000b1"),
        ];
        let summaries = [hook_summary(SUMMARY_UUID)];
        for _ in 0..2 {
            write_batch(
                db.store_mut(),
                &request(&context, &records, &summaries, None, keep_content),
            )
            .unwrap();
        }
        assert_eq!(
            scalar(
                db.path(),
                "SELECT count(*) FROM tool_uses WHERE input_json IS NOT NULL"
            ),
            if keep_content { 6 } else { 0 },
            "keep_content={keep_content}"
        );
        assert_eq!(
            scalar(
                db.path(),
                "SELECT count(*) FROM records WHERE content_json IS NOT NULL"
            ),
            if keep_content { 2 } else { 0 }
        );
        // Counts and kinds are identical in both modes, and replay adds nothing.
        assert_eq!(scalar(db.path(), "SELECT count(*) FROM tool_uses"), 8);
        assert_eq!(db.store().tool_events(SESSION).unwrap().len(), 2);
        structures.push(kinds(db.store(), SESSION));
    }
    assert_eq!(structures[0], structures[1]);
}

#[test]
fn a_legacy_row_acquires_its_classification_only_from_a_compatible_replay() {
    let mut db = TempDb::empty().unwrap();
    let context = context(SESSION, SessionSource::Transcript);
    let records = [tool_call_record("00000000-0000-4000-8000-0000000000b1")];
    write_batch(
        db.store_mut(),
        &request(&context, &records, &[], None, false),
    )
    .unwrap();
    // Simulate rows an older build wrote: names and positions, no structure.
    Connection::open(db.path())
        .unwrap()
        .execute(
            "UPDATE tool_uses SET kind=NULL,server=NULL,tool=NULL,skill=NULL WHERE uuid IS NOT NULL",
            [],
        )
        .unwrap();
    let mut store = Store::open(db.path()).unwrap();
    assert!(kinds(&store, SESSION).iter().all(|row| row.1.is_none()));
    write_batch(&mut store, &request(&context, &records, &[], None, false)).unwrap();
    assert_eq!(
        kinds(&store, SESSION)
            .into_iter()
            .map(|row| row.1)
            .collect::<Vec<_>>(),
        [
            Some(ToolKind::Builtin),
            Some(ToolKind::Builtin),
            Some(ToolKind::Mcp),
            Some(ToolKind::Skill),
            Some(ToolKind::Subagent),
            Some(ToolKind::Subagent),
        ]
    );

    // A replay whose block identity disagrees is a conflict, not an overwrite.
    let renamed = parsed(
        json!({"uuid":"00000000-0000-4000-8000-0000000000b1","type":"assistant",
        "timestamp":"2026-09-07T12:00:01Z","message":{"role":"assistant","content":[
            {"type":"tool_use","name":"Write","input":{}},
            {"type":"tool_use","name":"Edit","input":{}},
            {"type":"tool_use","name":"mcp__Claude_Browser__computer","input":{}},
            {"type":"tool_use","name":"Skill","input":{}},
            {"type":"tool_use","name":"Agent","input":{}},
            {"type":"tool_use","name":"Task","input":{}}
        ]}}),
    );
    write_batch(&mut store, &request(&context, &[renamed], &[], None, false)).unwrap();
    let row = store.records(SESSION).unwrap().remove(0);
    assert!(row.has_conflict, "a differing tool name is a conflict");
    assert_eq!(row.tool_uses[0].name, "Bash", "the stored call is retained");
    assert_eq!(row.tool_uses[0].kind, Some(ToolKind::Builtin));
}

#[test]
fn f1_keeps_its_exact_tool_call_total_while_structural_rows_are_counted_apart() {
    let fixture = fixture("F1");
    let db = fixture.build_db(true).unwrap();
    // The golden itself, not a number restated here.
    let expected = fixture
        .expected()
        .iter()
        .find(|(rule, _)| rule.to_string() == "M-03")
        .expect("F1 states M-03")
        .1["tool_calls"]
        .as_i64()
        .unwrap();
    let session = &fixture.sessions()[0].metadata.session_id;
    let record_bound = scalar(
        db.path(),
        "SELECT count(*) FROM tool_uses WHERE uuid IS NOT NULL",
    );
    assert_eq!(record_bound, expected, "F1 tool_calls must not move");
    assert_eq!(
        db.store()
            .records(session)
            .unwrap()
            .iter()
            .map(|record| record.tool_uses.len())
            .sum::<usize>() as i64,
        expected
    );
    // Every F1 call is classified, and F1 states no structural event.
    assert!(
        kinds(db.store(), session)
            .iter()
            .all(|row| row.1 == Some(ToolKind::Builtin) && row.0 == "Read")
    );
    assert_eq!(
        scalar(
            db.path(),
            "SELECT count(*) FROM tool_uses WHERE uuid IS NULL"
        ),
        0
    );
    assert!(db.store().tool_events(session).unwrap().is_empty());

    // Structural rows added to the same session leave the call total alone.
    let mut db = db;
    let context = context(session, SessionSource::Transcript);
    let summaries = [hook_summary(SUMMARY_UUID)];
    write_batch(
        db.store_mut(),
        &request(&context, &[], &summaries, None, true),
    )
    .unwrap();
    assert_eq!(
        scalar(
            db.path(),
            "SELECT count(*) FROM tool_uses WHERE uuid IS NOT NULL"
        ),
        expected,
        "a structural event never inflates the tool-call total"
    );
    assert_eq!(db.store().tool_events(session).unwrap().len(), 1);
}

/// Only a user record states a slash command. A record that merely carries the
/// marker text is a lookalike: an assistant record quoting it, and a record
/// whose role is unstated, are not evidence the user invoked anything.
#[test]
fn only_an_explicit_user_role_states_a_slash_command() {
    let marker = json!([{"type":"text","text":"<command-name>/memhub:login</command-name>"}]);
    let lookalikes = [
        // An assistant record quoting the marker.
        json!({"uuid":"00000000-0000-4000-8000-0000000000d1","type":"assistant",
            "timestamp":"2026-09-07T12:00:03Z",
            "message":{"role":"assistant","content":marker}}),
        // A user-typed record with no role stated at all.
        json!({"uuid":"00000000-0000-4000-8000-0000000000d2","type":"user",
            "timestamp":"2026-09-07T12:00:04Z","message":{"content":marker}}),
        // A record whose type and role disagree with each other.
        json!({"uuid":"00000000-0000-4000-8000-0000000000d3","type":"assistant",
            "timestamp":"2026-09-07T12:00:05Z","message":{"role":"user","content":marker}}),
    ]
    .map(parsed);

    let mut db = TempDb::empty().unwrap();
    let context = context(SESSION, SessionSource::Transcript);
    write_batch(
        db.store_mut(),
        &request(&context, &lookalikes, &[], None, false),
    )
    .unwrap();
    assert!(
        db.store().tool_events(SESSION).unwrap().is_empty(),
        "a marker without an explicit user role states no command"
    );
    assert_eq!(scalar(db.path(), "SELECT count(*) FROM records"), 3);

    // The genuine user record in the same session still states its command.
    let genuine = [command_record(COMMAND_UUID)];
    write_batch(
        db.store_mut(),
        &request(&context, &genuine, &[], None, false),
    )
    .unwrap();
    let events = db.store().tool_events(SESSION).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source_event_id, COMMAND_UUID);
    assert_eq!(events[0].kind, ToolKind::Command);
}

/// A hook summary names the same native identity a record does, and is
/// reconciled by the same agreement rules before anything it implies is
/// persisted. A summary that disagrees commits neither its event nor the
/// checkpoint covering it.
#[test]
fn a_disagreeing_hook_summary_commits_neither_event_nor_checkpoint() {
    let disagreements = [
        // A native session that is not this batch's.
        json!({"type":"system","subtype":"stop_hook_summary","uuid":SUMMARY_UUID,
            "timestamp":"2026-09-07T12:00:02Z","sessionId":"00000000-0000-4000-8000-00000000ffff"}),
        // A surface that is not this batch's.
        json!({"type":"system","subtype":"stop_hook_summary","uuid":SUMMARY_UUID,
            "timestamp":"2026-09-07T12:00:02Z","source_surface":"desktop"}),
        // A platform that is not this batch's.
        json!({"type":"system","subtype":"stop_hook_summary","uuid":SUMMARY_UUID,
            "timestamp":"2026-09-07T12:00:02Z","source_platform":"codex"}),
        // A conversation identity that is not this batch's.
        json!({"type":"system","subtype":"stop_hook_summary","uuid":SUMMARY_UUID,
            "timestamp":"2026-09-07T12:00:02Z","conversation_id":"another-session"}),
    ]
    .map(summary);

    for (index, disagreeing) in disagreements.into_iter().enumerate() {
        let mut db = TempDb::empty().unwrap();
        let context = context(SESSION, SessionSource::Transcript);
        let records = [command_record(COMMAND_UUID)];
        let checkpoint = xt_store::batch::NativeCheckpoint {
            source: SessionSource::Transcript,
            cursor_key: "claude:synthetic".into(),
            generation: r#"{"kind":"file","ino":1}"#.into(),
            position: 120,
            updated_at: 100,
        };
        let mut batch = request(&context, &records, &[], None, false);
        let summaries = [disagreeing];
        batch.hook_summaries = &summaries;
        batch.checkpoint = Some(&checkpoint);
        assert!(
            write_batch(db.store_mut(), &batch).is_err(),
            "disagreement {index} must fail the batch"
        );
        assert_eq!(scalar(db.path(), "SELECT count(*) FROM tool_uses"), 0);
        assert_eq!(scalar(db.path(), "SELECT count(*) FROM records"), 0);
        assert!(
            db.store()
                .native_checkpoint(SessionSource::Transcript, "claude:synthetic")
                .unwrap()
                .is_none(),
            "disagreement {index} must commit no checkpoint"
        );
    }

    // An agreeing summary, with the same fields stated compatibly, commits.
    let mut db = TempDb::empty().unwrap();
    let context = context(SESSION, SessionSource::Transcript);
    let records = [command_record(COMMAND_UUID)];
    let agreeing = [summary(
        json!({"type":"system","subtype":"stop_hook_summary",
        "uuid":SUMMARY_UUID,"timestamp":"2026-09-07T12:00:02Z","sessionId":SESSION,
        "conversation_id":SESSION,"source_platform":"claude","source_surface":"cli"}),
    )];
    write_batch(
        db.store_mut(),
        &request(&context, &records, &agreeing, None, false),
    )
    .unwrap();
    assert_eq!(db.store().tool_events(SESSION).unwrap().len(), 2);
}

/// A rejected occurrence is not a trustworthy account of the record it claims to
/// be. Session A stores the UUID as an ordinary record; session B then presents
/// the same UUID carrying a command marker and is rejected for ownership. No
/// command event may appear under either session: not under B, which does not
/// own the record, and not under A, which never stored a command.
#[test]
fn a_rejected_occurrence_writes_no_command_onto_the_owning_session() {
    let mut db = TempDb::empty().unwrap();
    // A stores the UUID as a plain user record that states no command.
    let plain = [parsed(json!({"uuid":COMMAND_UUID,"type":"user",
        "timestamp":"2026-09-07T12:00:00Z",
        "message":{"role":"user","content":[{"type":"text","text":"An ordinary turn."}]}}))];
    write_batch(
        db.store_mut(),
        &request(
            &context(SESSION, SessionSource::Transcript),
            &plain,
            &[],
            None,
            false,
        ),
    )
    .unwrap();
    assert!(db.store().tool_events(SESSION).unwrap().is_empty());

    // B presents the same UUID with a command marker and is rejected.
    let claimed = [command_record(COMMAND_UUID)];
    write_batch(
        db.store_mut(),
        &request(
            &context(OTHER_SESSION, SessionSource::Transcript),
            &claimed,
            &[],
            None,
            false,
        ),
    )
    .unwrap();
    assert!(
        db.store().tool_events(OTHER_SESSION).unwrap().is_empty(),
        "the rejecting session states no command"
    );
    assert!(
        db.store().tool_events(SESSION).unwrap().is_empty(),
        "a rejected occurrence cannot write a command onto the owning session"
    );
    assert_eq!(scalar(db.path(), "SELECT count(*) FROM tool_uses"), 0);
    // A's record keeps what A stored; only the ownership conflict is recorded.
    assert_eq!(scalar(db.path(), "SELECT count(*) FROM records"), 1);
}
