use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::PathBuf;
use xt_fixtures::{Fixture, RuleId, TempDb};
use xt_metrics::{
    DayCalls, EnvUsage, Error, HostEnvironment, HostInventory, IdentityCalls, Inventory,
    InventoryJoin, MetricsDb, ToolIdentity, TypingRate, UnresolvedCalls, UnresolvedReason, Window,
};
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource,
    ingest::{ToolEvent, ToolKind},
    tool_use::HOOK_EVENT_NAME,
};

fn fixture(id: &str) -> Fixture {
    Fixture::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(id),
    )
    .unwrap()
}
fn ms(value: &str) -> i64 {
    value.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn report(db: &TempDb, inventory: &[HostInventory]) -> EnvUsage {
    MetricsDb::open(db.path())
        .unwrap()
        .environment(window(), TimeZone::UTC, inventory)
        .unwrap()
}
fn identity(
    kind: Option<ToolKind>,
    name: &str,
    server: Option<&str>,
    tool: Option<&str>,
    skill: Option<&str>,
) -> ToolIdentity {
    ToolIdentity {
        kind: kind.map(|kind| kind.as_str().to_owned()),
        name: name.to_owned(),
        server: server.map(str::to_owned),
        tool: tool.map(str::to_owned),
        skill: skill.map(str::to_owned),
    }
}
fn builtin(name: &str) -> ToolIdentity {
    identity(Some(ToolKind::Builtin), name, None, None, None)
}
/// The expected identity row over the default report window: one count per
/// local day, in window order, whose sum is the row's total. Stating the whole
/// series is the point — a caller reads a day's count off the row itself.
fn called(identity: ToolIdentity, by_day: &[u64]) -> IdentityCalls {
    called_in(window(), identity, by_day)
}
/// The same over an explicitly named UTC window.
fn called_in(window: Window, identity: ToolIdentity, by_day: &[u64]) -> IdentityCalls {
    let days = window.local_days(TimeZone::UTC).unwrap();
    assert_eq!(
        days.len(),
        by_day.len(),
        "one expected count per reported day"
    );
    IdentityCalls {
        identity,
        calls: by_day.iter().sum(),
        by_day: days
            .iter()
            .zip(by_day)
            .map(|(day, calls)| DayCalls {
                date: day.date.to_string(),
                start_ms: day.window.start_ms(),
                end_ms: day.window.end_ms(),
                calls: *calls,
            })
            .collect(),
    }
}
/// A whole default-window series of zeros: what an installed item that was
/// never called still reports.
const NEVER: [u64; 7] = [0; 7];
/// The default window's final local day, which is where these fixtures act.
const FINAL_DAY: usize = 6;
fn only_final_day(calls: u64) -> [u64; 7] {
    let mut series = NEVER;
    series[FINAL_DAY] = calls;
    series
}
fn known(host: &str, items: &[ToolIdentity]) -> Vec<HostInventory> {
    vec![HostInventory {
        host: host.to_owned(),
        installed: Inventory::Known(items.to_vec()),
    }]
}
/// One assistant record carrying the named tool-use blocks, and nothing else.
fn tools(uuid: &str, ts: Option<&str>, blocks: &[(&str, Value)]) -> CanonicalRecord {
    let content: Vec<Value> = blocks
        .iter()
        .enumerate()
        .map(|(index, (name, input))| {
            json!({"type":"tool_use","id":format!("{uuid}-{index}"),"name":name,"input":input})
        })
        .collect();
    let mut value =
        json!({"uuid":uuid,"type":"assistant","message":{"role":"assistant","content":content}});
    if let Some(ts) = ts {
        value["timestamp"] = json!(ts);
    }
    serde_json::from_value(value).unwrap()
}
fn seed(
    db: &mut TempDb,
    session_id: &str,
    platform: &str,
    surface: Option<&str>,
    records: &[CanonicalRecord],
) {
    let mut session = SessionMeta::new(session_id, platform, SessionSource::Fixture);
    session.surface = surface.map(str::to_owned);
    db.store_mut().upsert_session(&session, false).unwrap();
    db.store_mut()
        .upsert_records(session_id, records, false)
        .unwrap();
}
fn event(
    session_id: &str,
    source_event_id: &str,
    ts: Option<&str>,
    name: &str,
    kind: ToolKind,
) -> ToolEvent {
    ToolEvent {
        session_id: session_id.to_owned(),
        source: SessionSource::Fixture,
        source_event_id: source_event_id.to_owned(),
        timestamp: ts.map(str::to_owned),
        name: name.to_owned(),
        kind,
        server: None,
        tool: None,
        skill: None,
    }
}
fn days(report: &EnvUsage, index: usize) -> Vec<u64> {
    report.observed[index]
        .by_day
        .iter()
        .map(|day| day.calls)
        .collect()
}
fn unresolved(host: &str, reason: UnresolvedReason, calls: u64) -> UnresolvedCalls {
    UnresolvedCalls {
        host: host.to_owned(),
        reason,
        calls,
    }
}

#[test]
fn environment_f1_counts_each_stored_block_once_and_matches_the_tool_call_golden() {
    let f = fixture("F1");
    let db = f.build_db(false).unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let w = Window::new(
        f.window_start().timestamp_millis(),
        f.now().timestamp_millis(),
    )
    .unwrap();
    // F1's accepted M-03 golden counts the same five stored blocks, so the
    // environment total is pinned to an already-reviewed expectation.
    assert_eq!(
        f.expected()[&RuleId::parse("M-03").unwrap()]["tool_calls"],
        json!(5)
    );
    let r = metrics.environment(w, TimeZone::UTC, &[]).unwrap();
    assert_eq!(
        (r.window_start_ms, r.window_end_ms),
        (w.start_ms(), w.end_ms())
    );
    assert_eq!(r.observed.len(), 1);
    let surface = &r.observed[0];
    assert_eq!(
        (
            surface.host.as_str(),
            surface.surface.as_deref(),
            surface.calls
        ),
        ("claude", Some("cli"), 5)
    );
    assert_eq!(
        surface.by_identity,
        vec![called_in(w, builtin("Read"), &[0, 0, 0, 0, 0, 0, 5])]
    );
    assert_eq!(days(&r, 0), vec![0, 0, 0, 0, 0, 0, 5]);
    assert_eq!(surface.by_day[6].date, "2026-09-07");
    assert!(r.unresolved.is_empty());
    // No inventory was supplied, so installation stays unknown while the
    // observation is fully measured.
    assert_eq!(
        r.hosts,
        vec![HostEnvironment {
            host: "claude".into(),
            inventory: InventoryJoin::Unknown,
            unresolved_calls: 0,
        }]
    );
    assert_eq!(
        metrics.counts(w, TypingRate::default()).unwrap().tool_calls,
        Some(5)
    );
}

#[test]
fn environment_command_and_hook_are_separate_kinds_and_not_assistant_tool_calls() {
    let f = fixture("F1");
    let mut db = f.build_db(false).unwrap();
    let session = f.sessions()[0].metadata.session_id.clone();
    // A slash command is stated by the user record that carries this UUID.
    let backing = f.sessions()[0].records[0].uuid.clone().unwrap();
    for e in [
        event(
            &session,
            &backing,
            Some("2026-09-07T12:00:00Z"),
            "/loop",
            ToolKind::Command,
        ),
        // Two summaries, whatever number of hooks each one describes.
        event(
            &session,
            "hook-a",
            Some("2026-09-07T12:04:00Z"),
            HOOK_EVENT_NAME,
            ToolKind::Hook,
        ),
        event(
            &session,
            "hook-b",
            Some("2026-09-07T12:09:00Z"),
            HOOK_EVENT_NAME,
            ToolKind::Hook,
        ),
    ] {
        db.store_mut().insert_tool_event(&e).unwrap();
    }
    let r = report(&db, &[]);
    assert_eq!(r.observed[0].calls, 8);
    assert_eq!(
        r.observed[0].by_identity,
        vec![
            called(builtin("Read"), &only_final_day(5)),
            called(
                identity(Some(ToolKind::Command), "/loop", None, None, None),
                &only_final_day(1),
            ),
            called(
                identity(Some(ToolKind::Hook), HOOK_EVENT_NAME, None, None, None),
                &only_final_day(2),
            ),
        ]
    );
    // Structural events are not assistant tool calls and never join that total.
    assert_eq!(
        MetricsDb::open(db.path())
            .unwrap()
            .counts(window(), TypingRate::default())
            .unwrap()
            .tool_calls,
        Some(5)
    );
    let raw = Connection::open(db.path()).unwrap();
    assert_eq!(
        raw.query_row(
            "SELECT sum(tool_use_count) FROM records WHERE session_id=?1",
            [&session],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        5
    );
}

#[test]
fn environment_metadata_only_and_content_modes_report_identical_calls() {
    let f = fixture("F1");
    let metadata = f.build_db(false).unwrap();
    let content = f.build_db(true).unwrap();
    let stored = |db: &TempDb| -> (i64, i64) {
        let raw = Connection::open(db.path()).unwrap();
        (
            raw.query_row(
                "SELECT count(*) FROM tool_uses WHERE input_json IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap(),
            raw.query_row(
                "SELECT count(*) FROM records WHERE content_json IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap(),
        )
    };
    assert_eq!(stored(&metadata), (0, 0));
    assert_eq!(stored(&content), (5, 25));
    let with_content = report(&content, &[]);
    assert_eq!(report(&metadata, &[]), with_content);
    // Removing every stored input and content payload changes nothing, so no
    // count here can be reading tool input or transcript text.
    Connection::open(content.path())
        .unwrap()
        .execute_batch("UPDATE tool_uses SET input_json=NULL; UPDATE records SET content_json=NULL")
        .unwrap();
    assert_eq!(stored(&content), (0, 0));
    assert_eq!(report(&content, &[]), with_content);
}

#[test]
fn environment_replay_copies_and_response_dedupe_never_change_the_count() {
    let f = fixture("F1");
    let mut db = f.build_db(false).unwrap();
    let session = f.sessions()[0].metadata.session_id.clone();
    let summary = event(
        &session,
        "hook-a",
        Some("2026-09-07T12:04:00Z"),
        HOOK_EVENT_NAME,
        ToolKind::Hook,
    );
    db.store_mut().insert_tool_event(&summary).unwrap();
    let before = report(&db, &[]);
    assert_eq!(before.observed[0].calls, 6);
    // Replaying the identical records and the identical structural identity.
    db.store_mut()
        .upsert_records(&session, &f.sessions()[0].records, false)
        .unwrap();
    db.store_mut().insert_tool_event(&summary).unwrap();
    assert_eq!(report(&db, &[]), before);
    // A native fork repeating the same records in another context file.
    Connection::open(db.path()).unwrap().execute(
        "INSERT INTO native_record_copies(session_id,record_uuid) SELECT session_id,uuid FROM records WHERE session_id=?1",
        [&session],
    ).unwrap();
    assert_eq!(report(&db, &[]), before);
    // Two snapshots of one API response, each carrying its own block. Selecting
    // one usage row per response would lose a block; both are stored work.
    let usage = json!({"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0});
    let mut snapshots = Vec::new();
    for (uuid, ts, name) in [
        ("snap-early", "2026-09-07T13:00:00Z", "Grep"),
        ("snap-late", "2026-09-07T13:00:01Z", "Glob"),
    ] {
        let mut record = tools(uuid, Some(ts), &[(name, json!({}))]);
        record.api_message_id = Some("shared-response".into());
        record.request_id = Some("shared-request".into());
        record.message.usage = serde_json::from_value(usage.clone()).unwrap();
        snapshots.push(record);
    }
    db.store_mut()
        .upsert_records(&session, &snapshots, false)
        .unwrap();
    let raw = Connection::open(db.path()).unwrap();
    xt_store::timestamp::register_sqlite(&raw).unwrap();
    assert_eq!(
        raw.query_row(
            "SELECT count(*) FROM v_response_usage WHERE api_message_id='shared-response'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let after = report(&db, &[]);
    assert_eq!(after.observed[0].calls, 8);
    assert_eq!(
        after.observed[0].by_identity,
        vec![
            called(builtin("Glob"), &only_final_day(1)),
            called(builtin("Grep"), &only_final_day(1)),
            called(builtin("Read"), &only_final_day(5)),
            called(
                identity(Some(ToolKind::Hook), HOOK_EVENT_NAME, None, None, None),
                &only_final_day(1),
            ),
        ]
    );
}

#[test]
fn environment_precise_membership_keeps_a_leap_second_before_midnight() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "leap",
        "claude",
        Some("cli"),
        &[
            tools(
                "inside",
                Some("2026-09-07T23:59:59Z"),
                &[("Read", json!({}))],
            ),
            tools(
                "leap",
                Some("2026-09-07T23:59:60.9Z"),
                &[("Read", json!({}))],
            ),
            tools("next", Some("2026-09-08T00:00:00Z"), &[("Read", json!({}))]),
            tools(
                "beyond",
                Some("2026-09-08T00:00:60Z"),
                &[("Read", json!({}))],
            ),
        ],
    );
    db.store_mut()
        .insert_tool_event(&event(
            "leap",
            "hook-leap",
            Some("2026-09-07T23:59:60.9Z"),
            HOOK_EVENT_NAME,
            ToolKind::Hook,
        ))
        .unwrap();
    let r = report(&db, &[]);
    assert_eq!(r.observed[0].calls, 3);
    assert_eq!(days(&r, 0), vec![0, 0, 0, 0, 0, 0, 3]);
    // The adjacent window holds exactly the midnight call, so neither the leap
    // instant nor the second beyond it is counted twice or lost.
    let next = Window::new(window().end_ms(), window().end_ms() + 1000).unwrap();
    let following = MetricsDb::open(db.path())
        .unwrap()
        .environment(next, TimeZone::UTC, &[])
        .unwrap();
    assert_eq!(following.observed[0].calls, 1);
    assert_eq!(
        following.observed[0].by_identity,
        vec![called_in(next, builtin("Read"), &[1])]
    );
    assert!(following.unresolved.is_empty());
}

#[test]
fn environment_local_day_buckets_follow_the_window_across_a_dst_change() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "dst",
        "claude",
        Some("cli"),
        &[
            tools(
                "before",
                Some("2026-03-08T08:00:00Z"),
                &[("Read", json!({}))],
            ),
            tools(
                "after",
                Some("2026-03-09T07:00:00Z"),
                &[("Read", json!({}))],
            ),
        ],
    );
    let w = Window::new(
        ms("2026-03-07T12:00:00-08:00"),
        ms("2026-03-10T12:00:00-07:00"),
    )
    .unwrap();
    assert_eq!(w.end_ms() - w.start_ms(), 71 * 60 * 60 * 1000);
    let r = MetricsDb::open(db.path())
        .unwrap()
        .environment(w, TimeZone::get("America/Los_Angeles").unwrap(), &[])
        .unwrap();
    assert_eq!(days(&r, 0), vec![0, 1, 1, 0]);
    assert_eq!(
        r.observed[0]
            .by_day
            .iter()
            .map(|day| day.date.as_str())
            .collect::<Vec<_>>(),
        ["2026-03-07", "2026-03-08", "2026-03-09", "2026-03-10"]
    );
}

#[test]
fn environment_unknown_kind_detail_and_surface_stay_visible_and_unresolved() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "unknown",
        "claude",
        None,
        &[tools(
            "row",
            Some("2026-09-07T12:00:00Z"),
            &[
                ("Legacy", json!({})),
                ("mcp__server", json!({})),
                ("mcp__server__tool", json!({})),
                ("Skill", json!({})),
            ],
        )],
    );
    // A row stored before ingest classified kinds keeps its unknown kind.
    Connection::open(db.path())
        .unwrap()
        .execute("UPDATE tool_uses SET kind=NULL WHERE name='Legacy'", [])
        .unwrap();
    let installed = identity(Some(ToolKind::Builtin), "Write", None, None, None);
    let r = report(&db, &known("claude", std::slice::from_ref(&installed)));
    assert_eq!(r.observed[0].surface, None);
    assert_eq!(
        r.observed[0].by_identity,
        vec![
            called(
                identity(None, "Legacy", None, None, None),
                &only_final_day(1),
            ),
            called(
                identity(Some(ToolKind::Mcp), "mcp__server", None, None, None),
                &only_final_day(1),
            ),
            called(
                identity(
                    Some(ToolKind::Mcp),
                    "mcp__server__tool",
                    Some("server"),
                    Some("tool"),
                    None
                ),
                &only_final_day(1),
            ),
            called(
                identity(Some(ToolKind::Skill), "Skill", None, None, None),
                &only_final_day(1),
            ),
        ]
    );
    assert_eq!(
        r.unresolved,
        vec![
            unresolved("claude", UnresolvedReason::UnknownKind, 1),
            unresolved("claude", UnresolvedReason::UnknownMcpDetail, 1),
            unresolved("claude", UnresolvedReason::UnknownSkillName, 1),
        ]
    );
    // Three calls cannot be compared with an item, so the installed zero below
    // is "no matched call in this window", not "never called".
    assert_eq!(r.hosts[0].unresolved_calls, 3);
    assert_eq!(
        r.hosts[0].inventory,
        InventoryJoin::Known {
            installed: vec![called(installed, &NEVER)],
            called_not_installed: vec![called(
                identity(
                    Some(ToolKind::Mcp),
                    "mcp__server__tool",
                    Some("server"),
                    Some("tool"),
                    None
                ),
                &only_final_day(1),
            )],
        }
    );
}

#[test]
fn environment_known_empty_inventory_is_measured_and_unknown_inventory_is_not() {
    let f = fixture("F1");
    let db = f.build_db(false).unwrap();
    assert_eq!(
        report(&db, &known("claude", &[])).hosts[0].inventory,
        InventoryJoin::Known {
            installed: Vec::new(),
            called_not_installed: vec![called(builtin("Read"), &only_final_day(5))],
        }
    );
    for inventory in [
        Vec::new(),
        vec![HostInventory {
            host: "claude".into(),
            installed: Inventory::Unknown,
        }],
    ] {
        let r = report(&db, &inventory);
        assert_eq!(r.hosts[0].inventory, InventoryJoin::Unknown);
        // The observation is unaffected by what the inventory owner knows.
        assert_eq!(r.observed[0].calls, 5);
    }
    let duplicated = [
        HostInventory {
            host: "claude".into(),
            installed: Inventory::Known(Vec::new()),
        },
        HostInventory {
            host: "claude".into(),
            installed: Inventory::Unknown,
        },
    ];
    assert!(matches!(
        MetricsDb::open(db.path())
            .unwrap()
            .environment(window(), TimeZone::UTC, &duplicated),
        Err(Error::DuplicateInventoryHost)
    ));
}

#[test]
fn environment_installed_zero_is_a_statement_about_this_window_only() {
    let f = fixture("F1");
    let db = f.build_db(false).unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let never = identity(
        Some(ToolKind::Mcp),
        "mcp__memhub__search",
        Some("memhub"),
        Some("search"),
        None,
    );
    // A repeated supplied entry is one installed item, credited its calls once.
    let inventory = known("claude", &[builtin("Read"), builtin("Read"), never.clone()]);
    let r = metrics
        .environment(window(), TimeZone::UTC, &inventory)
        .unwrap();
    assert_eq!(
        r.hosts[0].inventory,
        InventoryJoin::Known {
            installed: vec![
                called(builtin("Read"), &only_final_day(5)),
                called(never.clone(), &NEVER),
            ],
            called_not_installed: Vec::new(),
        }
    );
    assert_eq!(r.hosts[0].unresolved_calls, 0);
    // The identical inventory over an earlier window reports the called item as
    // zero too, which is why the report carries the window it measured.
    let previous = window().previous().unwrap();
    let earlier = metrics
        .environment(previous, TimeZone::UTC, &inventory)
        .unwrap();
    assert_eq!(
        (earlier.window_start_ms, earlier.window_end_ms),
        (previous.start_ms(), previous.end_ms())
    );
    assert!(earlier.observed.is_empty());
    assert_eq!(
        earlier.hosts[0].inventory,
        InventoryJoin::Known {
            installed: vec![
                called_in(previous, builtin("Read"), &NEVER),
                called_in(previous, never, &NEVER),
            ],
            called_not_installed: Vec::new(),
        }
    );
}

#[test]
fn environment_applies_work_exclusions_and_reports_untimed_calls() {
    let mut db = TempDb::empty().unwrap();
    let block = [("Read", json!({}))];
    seed(
        &mut db,
        "work",
        "claude",
        Some("cli"),
        &[
            tools("counted", Some("2026-09-07T12:00:00Z"), &block),
            tools("meta", Some("2026-09-07T12:01:00Z"), &block),
            tools("synthetic", Some("2026-09-07T12:02:00Z"), &block),
            tools("untimed", None, &block),
        ],
    );
    seed(
        &mut db,
        "judged",
        "claude",
        Some("cli"),
        &[tools("judge-call", Some("2026-09-07T12:03:00Z"), &block)],
    );
    for e in [
        // Backed by an excluded record, so the record decides.
        event(
            "work",
            "meta",
            Some("2026-09-07T12:01:00Z"),
            "/meta",
            ToolKind::Command,
        ),
        // Standalone summaries: only the owning session's kind can exclude them.
        event(
            "work",
            "hook-timed",
            Some("2026-09-07T12:05:00Z"),
            HOOK_EVENT_NAME,
            ToolKind::Hook,
        ),
        event(
            "work",
            "hook-untimed",
            None,
            HOOK_EVENT_NAME,
            ToolKind::Hook,
        ),
        event(
            "judged",
            "hook-judged",
            Some("2026-09-07T12:06:00Z"),
            HOOK_EVENT_NAME,
            ToolKind::Hook,
        ),
    ] {
        db.store_mut().insert_tool_event(&e).unwrap();
    }
    let raw = Connection::open(db.path()).unwrap();
    raw.execute("UPDATE records SET is_meta=1 WHERE uuid='meta'", [])
        .unwrap();
    raw.execute(
        "UPDATE records SET model='<synthetic>' WHERE uuid='synthetic'",
        [],
    )
    .unwrap();
    raw.execute(
        "UPDATE sessions SET kind='judge' WHERE session_id='judged'",
        [],
    )
    .unwrap();
    let r = report(&db, &[]);
    assert_eq!(r.observed.len(), 1);
    assert_eq!(
        r.observed[0].by_identity,
        vec![
            called(builtin("Read"), &only_final_day(1)),
            called(
                identity(Some(ToolKind::Hook), HOOK_EVENT_NAME, None, None, None),
                &only_final_day(1),
            ),
        ]
    );
    // The excluded rows are not work, so they raise no reason; the two rows
    // that are work but state no time are reported with theirs, and the one
    // counted summary is unresolved because it names no installed hook.
    assert_eq!(
        r.unresolved,
        vec![
            unresolved("claude", UnresolvedReason::MissingTimestamp, 2),
            unresolved("claude", UnresolvedReason::HookAttribution, 1),
        ]
    );
    assert_eq!(r.hosts[0].unresolved_calls, 3);
    // A call with no time belongs to no window, so it is reported as
    // unresolved by every window rather than silently landing in one. The
    // summary is timed, so only its own window raises its reason.
    let earlier = MetricsDb::open(db.path())
        .unwrap()
        .environment(window().previous().unwrap(), TimeZone::UTC, &[])
        .unwrap();
    assert!(earlier.observed.is_empty());
    assert_eq!(
        earlier.unresolved,
        vec![unresolved("claude", UnresolvedReason::MissingTimestamp, 2)]
    );
}

/// The MET-13 extension the TEST-PLAN names for F18: the same identities under
/// metadata-only storage and duplicate delivery keep identical counts. This
/// covers that extension only, not F18's C-02/C-08/P-01 acceptance.
#[test]
fn environment_f18_metadata_replay_in_both_orders_keeps_identical_counts() {
    let f = fixture("F18");
    let input = &f.manifest().sessions[0];
    let platform = input.source_platform.clone().unwrap();
    let surface = input.source_surface.clone().unwrap();
    let session = input.session_id.clone();
    let record = tools(
        "f18-call",
        Some("2026-09-07T12:00:00Z"),
        &[("mcp__memhub__search", json!({}))],
    );
    let summary = event(
        &session,
        "f18-hook",
        Some("2026-09-07T12:01:00Z"),
        HOOK_EVENT_NAME,
        ToolKind::Hook,
    );
    let mut expected = None;
    for plugin_first in [true, false] {
        let mut db = TempDb::empty().unwrap();
        seed(
            &mut db,
            &session,
            &platform,
            Some(&surface),
            std::slice::from_ref(&record),
        );
        if plugin_first {
            db.store_mut().insert_tool_event(&summary).unwrap();
            db.store_mut()
                .upsert_records(&session, std::slice::from_ref(&record), false)
                .unwrap();
        } else {
            db.store_mut()
                .upsert_records(&session, std::slice::from_ref(&record), false)
                .unwrap();
            db.store_mut().insert_tool_event(&summary).unwrap();
        }
        // Enriching the existing UUID with usage it did not carry before.
        let mut enriched = record.clone();
        enriched.message.usage = serde_json::from_value(
            json!({"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}),
        )
        .unwrap();
        db.store_mut()
            .upsert_records(&session, &[enriched], false)
            .unwrap();
        let r = report(&db, &known("cursor", &[]));
        assert_eq!(
            (
                r.observed[0].host.as_str(),
                r.observed[0].surface.as_deref()
            ),
            ("cursor", Some(surface.as_str()))
        );
        assert_eq!(
            r.observed[0].by_identity,
            vec![
                called(
                    identity(Some(ToolKind::Hook), HOOK_EVENT_NAME, None, None, None),
                    &only_final_day(1),
                ),
                called(
                    identity(
                        Some(ToolKind::Mcp),
                        "mcp__memhub__search",
                        Some("memhub"),
                        Some("search"),
                        None
                    ),
                    &only_final_day(1),
                ),
            ]
        );
        let raw = Connection::open(db.path()).unwrap();
        assert_eq!(
            raw.query_row(
                "SELECT count(*) FROM tool_uses WHERE input_json IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        match &expected {
            None => expected = Some(r),
            Some(first) => assert_eq!(&r, first),
        }
    }
}

/// The MET-13 extension the TEST-PLAN names for F20: an independent new raw
/// surface groups on its own without coercing or aliasing any other surface,
/// while installation stays a host fact. This covers that extension only, not
/// F20's O-11/O-12/M-18/C-08 acceptance.
#[test]
fn environment_f20_new_raw_surface_groups_independently_of_installation() {
    let f = fixture("F20");
    let discovery = f.snapshots()["coverage"]["discovery"].as_array().unwrap();
    let surfaces: Vec<Option<String>> = ["claude-cli", "future.app"]
        .iter()
        .map(|wanted| {
            discovery
                .iter()
                .find_map(|row| {
                    let surface = row["surface"].as_str()?;
                    (surface == *wanted).then(|| surface.to_owned())
                })
                .map(Some)
                .unwrap()
        })
        .chain([None])
        .collect();
    let mut db = TempDb::empty().unwrap();
    for (index, surface) in surfaces.iter().enumerate() {
        seed(
            &mut db,
            &format!("f20-{index}"),
            "claude",
            surface.as_deref(),
            &[tools(
                &format!("f20-call-{index}"),
                Some("2026-09-07T12:00:00Z"),
                &[("Read", json!({}))],
            )],
        );
    }
    let r = report(&db, &known("claude", &[builtin("Read")]));
    // A legacy import without a surface, the known CLI and the newly named
    // surface are three groups, each keeping the raw value it was given.
    assert_eq!(
        r.observed
            .iter()
            .map(|group| (group.surface.clone(), group.calls))
            .collect::<Vec<_>>(),
        vec![
            (None, 1),
            (Some("claude-cli".into()), 1),
            (Some("future.app".into()), 1),
        ]
    );
    // Installation is not surface-specific: one host row holds all three.
    assert_eq!(r.hosts.len(), 1);
    assert_eq!(
        r.hosts[0].inventory,
        InventoryJoin::Known {
            installed: vec![called(builtin("Read"), &only_final_day(3))],
            called_not_installed: Vec::new(),
        }
    );
    assert!(r.unresolved.is_empty());
}

#[test]
fn environment_composes_inside_one_caller_read_snapshot() {
    let f = fixture("F1");
    let db = f.build_db(false).unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let writer = Connection::open(db.path()).unwrap();
    // Both reports are composed against the snapshot the caller opened, so a
    // commit landing between them cannot change one and not the other.
    let (first, second) = metrics
        .read_snapshot(|db| {
            let first = db.environment(window(), TimeZone::UTC, &[])?;
            writer.execute("DELETE FROM tool_uses", [])?;
            Ok((first, db.environment(window(), TimeZone::UTC, &[])?))
        })
        .unwrap();
    assert_eq!(first.observed[0].calls, 5);
    assert_eq!(first, second);
    // The commit really landed; only the snapshot was holding it back.
    assert!(report(&db, &[]).observed.is_empty());
}

/// Two identities that were active on different local days, observed on two
/// surfaces of one host. Every identity row carries the whole day series, and
/// the inventory join aggregates those days across the host's surfaces.
#[test]
fn environment_identity_day_counts_split_two_identities_across_surfaces_and_days() {
    let search = identity(
        Some(ToolKind::Mcp),
        "mcp__memhub__search_memory",
        Some("memhub"),
        Some("search_memory"),
        None,
    );
    let mut db = TempDb::empty().unwrap();
    // Monday and Friday on the CLI, Wednesday and Friday on the web surface.
    seed(
        &mut db,
        "two-cli",
        "claude",
        Some("cli"),
        &[
            tools(
                "cli-mon",
                Some("2026-09-01T09:00:00Z"),
                &[("Read", json!({})), ("Read", json!({}))],
            ),
            tools(
                "cli-wed",
                Some("2026-09-03T09:00:00Z"),
                &[("mcp__memhub__search_memory", json!({}))],
            ),
            tools(
                "cli-fri",
                Some("2026-09-05T09:00:00Z"),
                &[("Read", json!({}))],
            ),
        ],
    );
    seed(
        &mut db,
        "two-web",
        "claude",
        Some("web"),
        &[tools(
            "web-fri",
            Some("2026-09-05T21:00:00Z"),
            &[
                ("mcp__memhub__search_memory", json!({})),
                ("mcp__memhub__search_memory", json!({})),
            ],
        )],
    );
    let never = builtin("Write");
    let r = report(
        &db,
        &known("claude", &[builtin("Read"), search.clone(), never.clone()]),
    );
    // Two surfaces of one host, each identity keeping its own day series.
    assert_eq!(
        r.observed
            .iter()
            .map(|group| (group.host.as_str(), group.surface.as_deref()))
            .collect::<Vec<_>>(),
        vec![("claude", Some("cli")), ("claude", Some("web"))]
    );
    assert_eq!(days(&r, 0), vec![2, 0, 1, 0, 1, 0, 0]);
    assert_eq!(
        r.observed[0].by_identity,
        vec![
            called(builtin("Read"), &[2, 0, 0, 0, 1, 0, 0]),
            called(search.clone(), &[0, 0, 1, 0, 0, 0, 0]),
        ]
    );
    assert_eq!(days(&r, 1), vec![0, 0, 0, 0, 2, 0, 0]);
    assert_eq!(
        r.observed[1].by_identity,
        vec![called(search.clone(), &[0, 0, 0, 0, 2, 0, 0])]
    );
    // Installation is a host fact, so the join sums the same days over both
    // surfaces, and the item that was never called still reports every day.
    assert_eq!(r.hosts.len(), 1);
    assert_eq!(
        r.hosts[0].inventory,
        InventoryJoin::Known {
            installed: vec![
                called(builtin("Read"), &[2, 0, 0, 0, 1, 0, 0]),
                called(never, &NEVER),
                called(search, &[0, 0, 1, 0, 2, 0, 0]),
            ],
            called_not_installed: Vec::new(),
        }
    );
    assert!(r.unresolved.is_empty());
    // Every row of every shape reports the same days, in the same order, and
    // sums to its own total, so no consumer has a count left to compute.
    let expected: Vec<(String, i64, i64)> = window()
        .local_days(TimeZone::UTC)
        .unwrap()
        .iter()
        .map(|day| {
            (
                day.date.to_string(),
                day.window.start_ms(),
                day.window.end_ms(),
            )
        })
        .collect();
    assert_eq!(
        expected
            .iter()
            .map(|day| day.0.as_str())
            .collect::<Vec<_>>(),
        [
            "2026-09-01",
            "2026-09-02",
            "2026-09-03",
            "2026-09-04",
            "2026-09-05",
            "2026-09-06",
            "2026-09-07"
        ]
    );
    let InventoryJoin::Known { installed, .. } = &r.hosts[0].inventory else {
        panic!("inventory was supplied");
    };
    for row in r
        .observed
        .iter()
        .flat_map(|group| group.by_identity.iter())
        .chain(installed)
    {
        assert_eq!(
            row.by_day
                .iter()
                .map(|day| (day.date.clone(), day.start_ms, day.end_ms))
                .collect::<Vec<_>>(),
            expected,
            "{:?}",
            row.identity
        );
        assert_eq!(
            row.calls,
            row.by_day.iter().map(|day| day.calls).sum::<u64>(),
            "{:?}",
            row.identity
        );
    }
}

/// A stop-hook summary is one observed, counted event, and it names no hook, so
/// it can be matched against no installed hook item: it is unresolved, and it
/// is offered as an executable nowhere in the inventory join.
#[test]
fn environment_hook_summaries_are_counted_but_never_matched_to_an_installed_hook() {
    let named = identity(Some(ToolKind::Hook), "memhub-capture", None, None, None);
    let summary = identity(Some(ToolKind::Hook), HOOK_EVENT_NAME, None, None, None);
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "hooks",
        "claude",
        Some("cli"),
        &[tools(
            "hook-host",
            Some("2026-09-07T12:00:00Z"),
            &[("Read", json!({}))],
        )],
    );
    // Three summaries over two local days, whatever hooks each one describes.
    for (id, ts) in [
        ("summary-tue", "2026-09-02T08:00:00Z"),
        ("summary-sun-a", "2026-09-06T08:00:00Z"),
        ("summary-sun-b", "2026-09-06T20:00:00Z"),
    ] {
        db.store_mut()
            .insert_tool_event(&event(
                "hooks",
                id,
                Some(ts),
                HOOK_EVENT_NAME,
                ToolKind::Hook,
            ))
            .unwrap();
    }
    // The registry states a real named hook, and — as a host registry may —
    // the summary name itself. Neither can be credited a summary.
    let r = report(
        &db,
        &known("claude", &[builtin("Read"), named.clone(), summary.clone()]),
    );
    // Observed and counted exactly like any other call, on its own days.
    assert_eq!(r.observed[0].calls, 4);
    assert_eq!(days(&r, 0), vec![0, 1, 0, 0, 0, 2, 1]);
    assert_eq!(
        r.observed[0].by_identity,
        vec![
            called(builtin("Read"), &only_final_day(1)),
            called(summary.clone(), &[0, 1, 0, 0, 0, 2, 0]),
        ]
    );
    // Unmatchable for inventory purposes, with its own reason, and counted in
    // this host's unresolved total.
    assert_eq!(
        r.unresolved,
        vec![unresolved("claude", UnresolvedReason::HookAttribution, 3)]
    );
    assert_eq!(r.hosts[0].unresolved_calls, 3);
    // The named hook reads 0 because a summary cannot credit it, and the
    // summary identity itself is never credited either.
    assert_eq!(
        r.hosts[0].inventory,
        InventoryJoin::Known {
            installed: vec![
                called(builtin("Read"), &only_final_day(1)),
                called(named, &NEVER),
                called(summary.clone(), &NEVER),
            ],
            called_not_installed: Vec::new(),
        }
    );
    // With a measured empty inventory the summary is still not presented as an
    // executable: it enters neither list, while the ordinary call does.
    let bare = report(&db, &known("claude", &[]));
    assert_eq!(
        bare.hosts[0].inventory,
        InventoryJoin::Known {
            installed: Vec::new(),
            called_not_installed: vec![called(builtin("Read"), &only_final_day(1))],
        }
    );
    assert_eq!(bare.unresolved, r.unresolved);
    assert_eq!(bare.observed, r.observed);
    // The summary is still visible as an observed identity, so it is counted
    // and never silently dropped.
    assert!(
        bare.observed[0]
            .by_identity
            .iter()
            .any(|row| row.identity == summary && row.calls == 3)
    );
}
