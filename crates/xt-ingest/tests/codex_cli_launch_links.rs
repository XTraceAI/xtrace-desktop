//! Sub-session links for Codex sessions a Codex agent started with a literal
//! fresh `codex exec --json` command, through the ordinary passes.
//!
//! Every file is written here: a synthetic Codex parent history holding one
//! launch in the shape of the real control (an `exec` cell whose one
//! `exec_command` runs the command, answered by its own first process result
//! while the run still goes on), and the launched Codex thread's own rollout.
//! Nothing real is read and no command runs.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use xt_ingest::native::{
    HostStatus,
    claude_launch::LaunchProgress,
    import_reader_lines,
    readers_cli::ReaderOutcome,
    session_creation::{SpawnBacklog, continue_codex_spawns, spawn_limits},
};
use xt_store::{
    Host, Store,
    child_fact::ChildEvidence,
    creation::{CreationEvidence, CreationWitness},
    session_list::{ParentEvidence, SessionFilter},
};

const T: i64 = 1_791_238_000_000;
const PARENT: &str = "01a00000-0000-7000-8000-0000000000aa";
const CHILD: &str = "01a10000-0000-7000-8000-0000000000c1";
const OTHER: &str = "01a10000-0000-7000-8000-0000000000c2";
const FIRST: &str = "0d000000-0000-4000-8000-0000000000f1";
const HANDLE: u64 = 69773;
/// The launch call's time; everything else is an offset from it.
const AT: i64 = 1_791_238_643_642;

fn iso(offset: i64) -> String {
    chrono::DateTime::from_timestamp_millis(AT + offset)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn js(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

struct Home {
    _temp: tempfile::TempDir,
    home: PathBuf,
    db: PathBuf,
}

impl Home {
    fn new() -> Self {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("home");
        fs::create_dir_all(home.join(".codex/sessions")).unwrap();
        fs::create_dir_all(home.join(".claude/projects")).unwrap();
        Self {
            db: root.join("index.sqlite"),
            _temp: temp,
            home,
        }
    }

    fn store(&self) -> Store {
        Store::open(&self.db).unwrap()
    }

    fn day(&self, day: &str) -> PathBuf {
        let path = self.home.join(".codex/sessions").join(day);
        fs::create_dir_all(&path).unwrap();
        path
    }
}

/// The real control's command, with an invented prompt.
const COMMAND: &str = "codex exec --json --ignore-user-config --ignore-rules --sandbox read-only \
    --skip-git-repo-check -m gpt-6-sol -c 'model_reasoning_effort=\"medium\"' \
    'Reply exactly SYNTHETIC-CONTROL. Do not use tools.'";

/// One `exec_command` operation running `command`, as the control's cell has it.
fn exec(command: &str) -> String {
    format!(
        "tools.exec_command({{cmd:{},workdir:\"/w\",sandbox_permissions:\"require_escalated\",\
         justification:\"Run the synthetic test.\",yield_time_ms:1000,max_output_tokens:2000}})",
        js(command)
    )
}

fn cell(commands: &[&str]) -> String {
    commands
        .iter()
        .map(|command| format!("text(await {});\n", exec(command)))
        .collect()
}

fn call(offset: i64, id: &str, input: &str) -> Value {
    json!({"timestamp": iso(offset), "type": "response_item", "payload": {
        "type": "custom_tool_call", "call_id": id, "name": "exec", "input": input}})
}

fn wait(offset: i64, id: &str, cell: &str) -> Value {
    json!({"timestamp": iso(offset), "type": "response_item", "payload": {
        "type": "function_call", "call_id": id, "name": "wait",
        "arguments": json!({"cell_id": cell, "yield_time_ms": 30000}).to_string()}})
}

fn output(offset: i64, id: &str, header: &str, results: &[Value]) -> Value {
    let mut items = vec![json!({"type": "input_text", "text": header})];
    items.extend(
        results
            .iter()
            .map(|result| json!({"type": "input_text", "text": result.to_string()})),
    );
    json!({"timestamp": iso(offset), "type": "response_item", "payload": {
        "type": "custom_tool_call_output", "call_id": id, "output": items}})
}

const DONE: &str = "Script completed\nWall time 7.3 seconds\nOutput:\n";

fn started(thread: &str) -> String {
    format!("{{\"type\":\"thread.started\",\"thread_id\":\"{thread}\"}}\n")
}

/// What the control's run had printed by its first result.
fn printed(thread: &str) -> String {
    format!(
        "Reading additional input from stdin...\n{}{{\"type\":\"turn.started\"}}\n",
        started(thread)
    )
}

/// The process still running, with what it had printed.
fn running(output: &str) -> Value {
    json!({"chunk_id": "6f9747", "wall_time_seconds": 1.001, "session_id": HANDLE,
        "original_token_count": 35, "output": output})
}

fn exited(code: i64, output: &str) -> Value {
    json!({"chunk_id": "d06517", "wall_time_seconds": 0.1, "exit_code": code, "output": output})
}

fn message(offset: i64, text: &str) -> Value {
    json!({"timestamp": iso(offset), "type": "response_item", "payload": {
        "type": "message", "role": "assistant",
        "content": [{"type": "output_text", "text": text}]}})
}

/// The control's launch: one operation, answered by its own running result.
fn launch_rows(command: &str, result: Value) -> Vec<Value> {
    vec![
        message(-2000, "Starting a synthetic conversation"),
        call(0, "call_launch", &cell(&[command])),
        output(7319, "call_launch", DONE, &[result]),
        message(9000, "After the launch"),
    ]
}

fn write_rows(path: &Path, rows: &[Value]) {
    let body: String = rows.iter().map(|row| format!("{row}\n")).collect();
    fs::write(path, body).unwrap();
}

/// A paginated Codex rollout: its header, then `rows` with consecutive
/// ordinals.
fn write_rollout(path: &Path, header: Value, body: &[Value]) {
    let mut rows = vec![header];
    for (index, row) in body.iter().enumerate() {
        let mut row = row.clone();
        row["ordinal"] = json!(index + 1);
        rows.push(row);
    }
    write_rows(path, &rows);
}

fn parent_path(home: &Home) -> PathBuf {
    home.day("2026/09/28")
        .join(format!("rollout-2026-09-28T16-14-04-{PARENT}.jsonl"))
}

fn write_parent(home: &Home, body: &[Value]) -> PathBuf {
    let path = parent_path(home);
    write_rollout(
        &path,
        json!({"timestamp": iso(-86_400_000), "type": "session_meta", "ordinal": 0,
            "payload": {"id": PARENT, "session_id": PARENT, "timestamp": iso(-86_400_000),
                "cwd": "/w", "originator": "Codex Desktop", "source": "vscode",
                "history_mode": "paginated"}}),
        body,
    );
    path
}

fn child_path(home: &Home, thread: &str) -> PathBuf {
    home.day("2026/10/05")
        .join(format!("rollout-2026-10-05T15-17-30-{thread}.jsonl"))
}

/// The child's own header as the control's has it: `source` `exec`, opened
/// after the launch, naming nothing else.
fn child_header(thread: &str) -> Value {
    json!({"timestamp": iso(6896), "type": "session_meta", "ordinal": 0,
        "payload": {"session_id": thread, "id": thread, "timestamp": iso(6766),
            "cwd": "/w", "originator": "Codex Desktop", "cli_version": "0.157.0",
            "source": "exec", "thread_source": "user", "model_provider": "openai",
            "history_mode": "paginated"}})
}

fn write_child(home: &Home, thread: &str, header: Value) -> PathBuf {
    let path = child_path(home, thread);
    write_rollout(
        &path,
        header,
        &[
            json!({"timestamp": iso(8294), "type": "response_item", "payload": {
                "type": "message", "role": "user", "id": "msg_synthetic",
                "content": [{"type": "input_text", "text": "Reply exactly SYNTHETIC-CONTROL."}]}}),
            message(9571, "SYNTHETIC-CONTROL"),
        ],
    );
    path
}

/// Index one Codex session as the reader does: its original rollout and, for
/// a child, its first real prompt.
fn index_codex(store: &mut Store, native: &str, path: &Path, first: Option<(&str, i64)>) {
    let mut lines = vec![
        json!({"type": "session", "host": "codex", "native_session_id": native,
               "conversation_id": format!("codex-{native}"), "source_surface": "codex_cli",
               "started_at": iso(-86_400_000), "cwd": "/w", "git_branch": null, "title": null,
               "path": path.display().to_string(), "mtime": 1791238643.0})
        .to_string(),
    ];
    let (uuid, offset) = first.unwrap_or(("5555aaaa-5555-4555-8555-000000000000", -2500));
    lines.push(
        json!({"uuid": uuid, "type": "user", "cwd": "/w", "timestamp": iso(offset),
               "message": {"role": "user", "content": [{"type": "text", "text": "Synthetic request"}]}})
        .to_string(),
    );
    let report = import_reader_lines(
        store,
        Host::Codex,
        "test".into(),
        lines.into_iter().map(Ok),
        T,
        || {
            Ok(ReaderOutcome {
                diagnostics: vec![],
                complete: true,
            })
        },
    );
    assert_eq!(report.status, HostStatus::Complete, "{report:?}");
}

fn index_child(store: &mut Store, home: &Home) {
    index_codex(store, CHILD, &child_path(home, CHILD), Some((FIRST, 8294)));
}

fn child_session(store: &Store) -> String {
    store
        .user_sessions_with_native(Host::Codex, CHILD)
        .unwrap()
        .into_iter()
        .next()
        .expect("child indexed")
}

fn settle(home: &Home, store: &mut Store, spawns: &mut SpawnBacklog) -> LaunchProgress {
    let mut total = LaunchProgress::default();
    for _ in 0..200 {
        let progress =
            continue_codex_spawns(store, &home.home, spawns, spawn_limits(), None, T).unwrap();
        let moved = progress.advanced();
        if let Some(pass) = progress.launches {
            total.files_read += pass.files_read;
            total.threads_unchanged += pass.threads_unchanged;
            total.launches_found += pass.launches_found;
            total.launches_broken += pass.launches_broken;
            total.linked += pass.linked;
            total.children += pass.children;
            total.unparented += pass.unparented;
            total.changed += pass.changed;
            total.rejected += pass.rejected;
            total.waiting += pass.waiting;
            total.bytes_read += pass.bytes_read;
        }
        if !moved {
            break;
        }
    }
    total
}

/// Human-eligible input records of a session, as the app's hours read them.
fn eligible(db: &Path, session: &str) -> i64 {
    rusqlite::Connection::open(db)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM v_human_inputs h JOIN records r ON r.uuid=h.uuid
             WHERE r.session_id=?1 AND h.human_is_eligible=1",
            [session],
            |row| row.get(0),
        )
        .unwrap()
}

fn shown_parent(store: &Store, child: &str) -> Option<(String, ParentEvidence)> {
    store
        .sessions_page_filtered(&SessionFilter::default(), None)
        .unwrap()
        .into_iter()
        .find(|row| row.id == child)
        .and_then(|row| row.parent)
        .map(|parent| (parent.session_id, parent.evidence))
}

/// One child fact: `(kind, source, launch call, first input, accepted)`.
type Fact = (ChildEvidence, String, Option<String>, Option<String>, bool);

/// Every child fact of `child`.
fn facts(store: &Store, child: &str) -> Vec<Fact> {
    store
        .child_facts(child)
        .unwrap()
        .into_iter()
        .map(|(fact, accepted)| {
            (
                fact.evidence_kind,
                fact.source_native_session_id,
                fact.launch_call_id,
                fact.first_record_uuid,
                accepted,
            )
        })
        .collect()
}

/// The control's shape end to end: the launch's own running result names
/// the child, the child is a fresh `exec` thread, and one ordinary start
/// records the child fact, then the relation; a restart changes nothing.
#[test]
fn links_the_control_shaped_fresh_exec_launch_and_stays_linked() {
    let home = Home::new();
    let parent = write_parent(&home, &launch_rows(COMMAND, running(&printed(CHILD))));
    write_child(&home, CHILD, child_header(CHILD));
    let mut store = home.store();
    index_codex(&mut store, PARENT, &parent, None);
    index_child(&mut store, &home);
    let child = child_session(&store);
    assert!(eligible(&home.db, &child) >= 1, "counted as typed before");
    let parent_before = eligible(&home.db, &format!("codex-{PARENT}"));

    let total = settle(&home, &mut store, &mut SpawnBacklog::starting());
    assert_eq!(total.launches_found, 1, "{total:?}");
    assert_eq!((total.children, total.linked), (1, 1), "{total:?}");
    assert_eq!(
        total.changed, 2,
        "the child fact, then the relation: {total:?}"
    );

    let (proof, conflicted) = store
        .codex_cli_launch_creation(&child)
        .unwrap()
        .expect("linked");
    assert!(!conflicted);
    assert_eq!(proof.parent_session_id, format!("codex-{PARENT}"));
    assert_eq!(proof.parent_native_session_id, PARENT);
    assert_eq!(proof.child_native_session_id, CHILD);
    assert_eq!(proof.first_record_uuid, FIRST);
    assert_eq!(proof.launch_call_id, "call_launch");
    assert_eq!(proof.launch_operation_index, 0);
    assert_eq!(proof.acknowledgment_call_id, "call_launch");
    assert_eq!(proof.process_session_id.as_deref(), Some("69773"));
    assert_eq!(proof.segment_rollout_id, PARENT);
    assert_eq!(
        (proof.launch_ordinal, proof.acknowledgment_ordinal),
        (Some(2), Some(3))
    );
    assert_eq!(proof.evidence_version, 1);
    let summary = store.claude_launch_summary().unwrap();
    assert_eq!(
        (
            summary.codex_relations_accepted,
            summary.relations_accepted,
            summary.candidates_codex,
            summary.candidates_linked
        ),
        (1, 0, 1, 1)
    );
    // Not a Claude launch relation.
    assert!(store.claude_launch_creation(&child).unwrap().is_none());
    let stored = store.session_creation(&child).unwrap().unwrap().0;
    assert_eq!(stored.evidence_kind, CreationEvidence::CodexCliLaunch);
    assert_eq!(stored.witness, CreationWitness::CodexExecJsonThreadStarted);
    assert_eq!(
        (stored.child_host, stored.parent_host),
        (Host::Codex, Host::Codex)
    );
    assert_eq!(
        facts(&store, &child),
        [(
            ChildEvidence::CodexCliLaunch,
            PARENT.to_owned(),
            Some("call_launch".to_owned()),
            Some(FIRST.to_owned()),
            true
        )]
    );
    assert_eq!(
        shown_parent(&store, &child),
        Some((format!("codex-{PARENT}"), ParentEvidence::AgentLaunch))
    );
    // The existing rule for every accepted relation; only the child changes.
    assert_eq!(eligible(&home.db, &child), 0);
    assert_eq!(
        eligible(&home.db, &format!("codex-{PARENT}")),
        parent_before
    );

    drop(store);
    let mut store = home.store();
    let again = settle(&home, &mut store, &mut SpawnBacklog::starting());
    assert_eq!(
        (again.files_read, again.changed, again.linked),
        (0, 0, 0),
        "{again:?}"
    );
    assert!(store.codex_cli_launch_creation(&child).unwrap().is_some());
}

/// The run's first result after a yield arrives in a wait on the launch's
/// running cell: that wait's output is the launch's own first result, and
/// it names the child.
#[test]
fn a_wait_on_the_launch_cell_holds_its_first_result() {
    let home = Home::new();
    let body = vec![
        call(0, "call_launch", &cell(&[COMMAND])),
        output(1000, "call_launch", "Script running with cell ID 12\n", &[]),
        wait(1100, "call_wait", "12"),
        output(7319, "call_wait", DONE, &[running(&printed(CHILD))]),
    ];
    let parent = write_parent(&home, &body);
    write_child(&home, CHILD, child_header(CHILD));
    let mut store = home.store();
    index_codex(&mut store, PARENT, &parent, None);
    index_child(&mut store, &home);
    let total = settle(&home, &mut store, &mut SpawnBacklog::starting());
    assert_eq!(total.linked, 1, "{total:?}");
    let (proof, _) = store
        .codex_cli_launch_creation(&child_session(&store))
        .unwrap()
        .unwrap();
    assert_eq!(proof.acknowledgment_call_id, "call_wait");
}

/// The child is known as an agent's child whether or not its parent is
/// indexed; the parent, indexed later, adds the relation without touching
/// the fact.
#[test]
fn the_child_is_known_without_its_parent_and_linked_once_it_is_indexed() {
    let home = Home::new();
    let parent = write_parent(&home, &launch_rows(COMMAND, running(&printed(CHILD))));
    write_child(&home, CHILD, child_header(CHILD));
    let mut store = home.store();
    index_child(&mut store, &home);
    let child = child_session(&store);
    let total = settle(&home, &mut store, &mut SpawnBacklog::starting());
    assert_eq!(
        (total.children, total.unparented, total.linked),
        (1, 1, 0),
        "{total:?}"
    );
    assert_eq!(facts(&store, &child).len(), 1);
    assert!(store.session_creation(&child).unwrap().is_none());
    assert_eq!(shown_parent(&store, &child), None);
    // The known-child bit rests on the fact alone.
    let context = xt_store::session_list::context(
        &rusqlite::Connection::open(&home.db).unwrap(),
        &[child.as_str()],
    )
    .unwrap();
    assert!(context[0].known_child, "{context:?}");

    index_codex(&mut store, PARENT, &parent, None);
    let mut spawns = SpawnBacklog::starting();
    spawns.launches.add_children([CHILD]);
    let later = settle(&home, &mut store, &mut spawns);
    assert_eq!(later.linked, 1, "{later:?}");
    assert_eq!(
        shown_parent(&store, &child),
        Some((format!("codex-{PARENT}"), ParentEvidence::AgentLaunch))
    );
    assert_eq!(
        facts(&store, &child).len(),
        1,
        "the same fact, not a second"
    );
}

/// A child indexed after its launch was looked at waits, and is linked
/// when a scan imports it.
#[test]
fn a_child_indexed_after_the_launch_is_linked_on_its_import() {
    let home = Home::new();
    let parent = write_parent(&home, &launch_rows(COMMAND, running(&printed(CHILD))));
    let mut store = home.store();
    index_codex(&mut store, PARENT, &parent, None);
    let mut spawns = SpawnBacklog::starting();
    let first = settle(&home, &mut store, &mut spawns);
    assert_eq!(
        (first.launches_found, first.waiting, first.linked),
        (1, 1, 0),
        "{first:?}"
    );
    write_child(&home, CHILD, child_header(CHILD));
    index_child(&mut store, &home);
    spawns.launches.add_children([CHILD]);
    spawns.launches.add_threads([CHILD]);
    let later = settle(&home, &mut store, &mut spawns);
    assert_eq!(later.linked, 1, "{later:?}");
}

/// Commands and results that are not a fresh start, or whose own first
/// result names no one thread, are no launch: nothing about the indexed
/// thread changes.
#[test]
fn refuses_every_launch_that_is_not_a_fresh_start_named_by_its_own_result() {
    let other_op = format!(
        "text(await {});\ntext(await {});\n",
        exec("git status"),
        exec(COMMAND)
    );
    let cases: Vec<(&str, Vec<Value>)> = vec![
        (
            "resume",
            launch_rows(
                &format!("codex exec --json resume {CHILD} 'More'"),
                running(&printed(CHILD)),
            ),
        ),
        (
            "fork",
            launch_rows(
                &format!("codex exec --json fork {OTHER}"),
                running(&printed(CHILD)),
            ),
        ),
        (
            "review",
            launch_rows("codex exec --json review", running(&printed(CHILD))),
        ),
        (
            "ephemeral",
            launch_rows(
                "codex exec --json --ephemeral 'x'",
                running(&printed(CHILD)),
            ),
        ),
        (
            "plain text output",
            launch_rows("codex exec 'x'", running(&format!("session id: {CHILD}\n"))),
        ),
        (
            "help",
            launch_rows("codex exec --json --help", exited(0, &started(CHILD))),
        ),
        (
            "an unknown option",
            launch_rows(
                "codex exec --json -i /w/a.png 'x'",
                running(&printed(CHILD)),
            ),
        ),
        (
            "a malformed command",
            launch_rows("codex exec --json 'x", running(&printed(CHILD))),
        ),
        (
            "events redirected away",
            launch_rows(
                "codex exec --json 'x' > /w/events.jsonl",
                running(&printed(CHILD)),
            ),
        ),
        (
            "another program printing the event",
            launch_rows(&format!("echo {CHILD}"), exited(0, &started(CHILD))),
        ),
        (
            "an identity only mentioned",
            launch_rows(
                &format!("codex exec --json 'Continue {CHILD}'"),
                running("Reading additional input from stdin...\n"),
            ),
        ),
        (
            "failed before a thread started",
            launch_rows(
                COMMAND,
                exited(1, "Error: unexpected argument '--bogus' found\n"),
            ),
        ),
        (
            "two threads",
            launch_rows(
                COMMAND,
                running(&format!("{}{}", started(CHILD), started(OTHER))),
            ),
        ),
        (
            "a later event whose type is cut",
            launch_rows(
                COMMAND,
                running(&format!("{}{{\"type\":\"turn.sta\n", started(CHILD))),
            ),
        ),
        (
            "a cut first start",
            launch_rows(
                COMMAND,
                running(&format!(
                    "{{\"type\":\"thread.started\",\"thread_id\":\"{}",
                    &CHILD[..20]
                )),
            ),
        ),
        (
            "a second start whose identity is cut",
            launch_rows(
                COMMAND,
                running(&format!(
                    "{}{{\"type\":\"thread.started\",\"thread_id\":\"01a1",
                    started(CHILD)
                )),
            ),
        ),
        (
            "another operation's result",
            vec![
                call(0, "call_launch", &other_op),
                output(
                    7319,
                    "call_launch",
                    DONE,
                    &[
                        exited(0, &started(CHILD)),
                        running("Reading additional input...\n"),
                    ],
                ),
            ],
        ),
        (
            "the thread only in a later poll",
            vec![
                call(0, "call_launch", &cell(&[COMMAND])),
                output(
                    1000,
                    "call_launch",
                    DONE,
                    &[running("Reading additional input...\n")],
                ),
                call(
                    5000,
                    "call_poll",
                    &format!(
                        "text(await tools.write_stdin({{session_id: {HANDLE}, chars: \"\", yield_time_ms: 30000}}))"
                    ),
                ),
                output(7319, "call_poll", DONE, &[running(&printed(CHILD))]),
            ],
        ),
    ];
    for (name, body) in cases {
        let home = Home::new();
        let parent = write_parent(&home, &body);
        write_child(&home, CHILD, child_header(CHILD));
        let mut store = home.store();
        index_codex(&mut store, PARENT, &parent, None);
        index_child(&mut store, &home);
        let child = child_session(&store);
        let total = settle(&home, &mut store, &mut SpawnBacklog::starting());
        // Not read as a launch at all.
        assert_eq!(total.launches_found, 0, "{name}: {total:?}");
        assert_eq!((total.children, total.linked), (0, 0), "{name}: {total:?}");
        assert!(facts(&store, &child).is_empty(), "{name}");
        assert!(store.session_creation(&child).unwrap().is_none(), "{name}");
        assert!(eligible(&home.db, &child) >= 1, "{name}");
    }
}

/// A named thread that is not a fresh `exec` session the launch opened is
/// not its child: another source, a fork or copied history, a header opened
/// before the launch, or no saved thread at all.
#[test]
fn refuses_a_named_thread_that_is_not_a_fresh_exec_child() {
    let header = |change: &dyn Fn(&mut Value)| {
        let mut header = child_header(CHILD);
        change(&mut header);
        header
    };
    let cases: Vec<(&str, Option<Value>)> = vec![
        (
            "an interactive source",
            Some(header(&|h| h["payload"]["source"] = json!("cli"))),
        ),
        (
            "no source",
            Some(header(&|h| {
                h["payload"].as_object_mut().unwrap().remove("source");
            })),
        ),
        (
            "opened before the launch",
            Some(header(&|h| h["payload"]["timestamp"] = json!(iso(-1)))),
        ),
        (
            "no opening time",
            Some(header(&|h| {
                h["payload"].as_object_mut().unwrap().remove("timestamp");
            })),
        ),
        (
            "a fork with copied history",
            Some(header(&|h| h["payload"]["forked_from_id"] = json!(OTHER))),
        ),
        (
            "a named parent",
            Some(header(&|h| {
                h["payload"]["parent_thread_id"] = json!(PARENT)
            })),
        ),
        (
            "another thread's header",
            Some(header(&|h| h["payload"]["id"] = json!(OTHER))),
        ),
        (
            "inherited rows",
            Some(header(&|h| {
                h["payload"]["subagent_history_start_ordinal"] = json!(1);
            })),
        ),
        ("no saved thread", None),
    ];
    for (name, header) in cases {
        let home = Home::new();
        let parent = write_parent(&home, &launch_rows(COMMAND, running(&printed(CHILD))));
        let mut store = home.store();
        index_codex(&mut store, PARENT, &parent, None);
        if let Some(header) = header {
            write_child(&home, CHILD, header);
            index_child(&mut store, &home);
        }
        let total = settle(&home, &mut store, &mut SpawnBacklog::starting());
        // The launch is found; its child is refused, or waited for.
        assert_eq!(total.launches_found, 1, "{name}: {total:?}");
        assert_eq!(
            (total.rejected, total.waiting),
            if name == "no saved thread" {
                (0, 1)
            } else {
                (1, 0)
            },
            "{name}: {total:?}"
        );
        assert_eq!((total.children, total.linked), (0, 0), "{name}: {total:?}");
        if let Some(child) = store
            .user_sessions_with_native(Host::Codex, CHILD)
            .unwrap()
            .first()
        {
            assert!(
                !facts(&store, child)
                    .iter()
                    .any(|fact| fact.0 == ChildEvidence::CodexCliLaunch),
                "{name}"
            );
            assert!(
                store.codex_cli_launch_creation(child).unwrap().is_none(),
                "{name}"
            );
        }
    }
}

/// The index's first eligible input must be the child's own and no earlier
/// than the launch; until the index holds one the launch waits.
#[test]
fn the_first_input_comes_from_the_index_and_follows_the_launch() {
    let home = Home::new();
    let parent = write_parent(&home, &launch_rows(COMMAND, running(&printed(CHILD))));
    write_child(&home, CHILD, child_header(CHILD));
    let mut store = home.store();
    index_codex(&mut store, PARENT, &parent, None);
    index_codex(
        &mut store,
        CHILD,
        &child_path(&home, CHILD),
        Some((FIRST, -5000)),
    );
    let total = settle(&home, &mut store, &mut SpawnBacklog::starting());
    assert_eq!(
        (total.children, total.linked, total.rejected),
        (0, 0, 1),
        "{total:?}"
    );
}

/// Two launches whose results both name one child: the child stays known by
/// both facts, and its relation is withheld, as for any two competing
/// creation anchors. A third, launched after the child opened, is refused.
#[test]
fn two_launches_naming_one_child_withhold_its_parent() {
    let home = Home::new();
    // Both launched before the child opened; a launch after it is no
    // creation at all.
    let body = vec![
        call(0, "call_launch", &cell(&[COMMAND])),
        call(100, "call_again", &cell(&[COMMAND])),
        output(7319, "call_launch", DONE, &[running(&printed(CHILD))]),
        output(7400, "call_again", DONE, &[running(&printed(CHILD))]),
        call(8000, "call_late", &cell(&[COMMAND])),
        output(8500, "call_late", DONE, &[running(&printed(CHILD))]),
    ];
    let parent = write_parent(&home, &body);
    write_child(&home, CHILD, child_header(CHILD));
    let mut store = home.store();
    index_codex(&mut store, PARENT, &parent, None);
    index_child(&mut store, &home);
    let child = child_session(&store);
    let total = settle(&home, &mut store, &mut SpawnBacklog::starting());
    assert_eq!((total.launches_found, total.rejected), (3, 1), "{total:?}");
    let (_, conflicted) = store.codex_cli_launch_creation(&child).unwrap().unwrap();
    assert!(conflicted, "{total:?}");
    assert_eq!(shown_parent(&store, &child), None);
    assert_eq!(facts(&store, &child).len(), 2);
    assert!(facts(&store, &child).iter().all(|fact| fact.4));
}

/// A thread whose own launch names itself is not its own child.
#[test]
fn a_result_naming_the_launching_thread_is_no_child() {
    let home = Home::new();
    let parent = write_parent(&home, &launch_rows(COMMAND, running(&printed(PARENT))));
    let mut store = home.store();
    index_codex(&mut store, PARENT, &parent, None);
    let total = settle(&home, &mut store, &mut SpawnBacklog::starting());
    assert_eq!((total.launches_found, total.children), (0, 0), "{total:?}");
}

/// A launch whose run printed a complete first start, then an output cut
/// inside a later event's body (or a malformed payload after its decoded
/// type), still names its child: only a later event's top-level type is
/// read.
#[test]
fn a_cut_later_event_leaves_the_first_start() {
    for later in [
        "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"text\":\"Long answ",
        "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,,}}\n",
    ] {
        let home = Home::new();
        let printed = format!("{}{later}", started(CHILD));
        let parent = write_parent(&home, &launch_rows(COMMAND, running(&printed)));
        write_child(&home, CHILD, child_header(CHILD));
        let mut store = home.store();
        index_codex(&mut store, PARENT, &parent, None);
        index_child(&mut store, &home);
        let total = settle(&home, &mut store, &mut SpawnBacklog::starting());
        assert_eq!((total.children, total.linked), (1, 1), "{later}: {total:?}");
    }
}

const SECOND_FIRST: &str = "0d000000-0000-4000-8000-0000000000f2";
const THIRD: &str = "01a10000-0000-7000-8000-0000000000c3";

/// A result naming `thread` from the running process `handle`.
fn running_as(handle: u64, thread: &str) -> Value {
    json!({"chunk_id": "6f9747", "wall_time_seconds": 1.001, "session_id": handle,
        "original_token_count": 35, "output": printed(thread)})
}

/// One cell starting two fresh runs, whose results come back as `body`
/// says; each child is linked to its own operation and answering call, an
/// unrelated output naming a third indexed thread links nothing, and a
/// restart changes nothing.
fn two_launches_in_one_cell(body: Vec<Value>, answered: [&str; 2]) {
    let home = Home::new();
    let parent = write_parent(&home, &body);
    let mut store = home.store();
    index_codex(&mut store, PARENT, &parent, None);
    for (thread, first) in [
        (CHILD, FIRST),
        (OTHER, SECOND_FIRST),
        (THIRD, "0d000000-0000-4000-8000-0000000000f3"),
    ] {
        write_child(&home, thread, child_header(thread));
        index_codex(
            &mut store,
            thread,
            &child_path(&home, thread),
            Some((first, 8294)),
        );
    }
    let session = |store: &Store, thread: &str| {
        store
            .user_sessions_with_native(Host::Codex, thread)
            .unwrap()
            .remove(0)
    };
    let total = settle(&home, &mut store, &mut SpawnBacklog::starting());
    assert_eq!(
        (total.launches_found, total.children, total.linked),
        (2, 2, 2),
        "{total:?}"
    );
    for (op, thread, first, handle) in [
        (0, CHILD, FIRST, "70001"),
        (1, OTHER, SECOND_FIRST, "70002"),
    ] {
        let child = session(&store, thread);
        let (proof, conflicted) = store
            .codex_cli_launch_creation(&child)
            .unwrap()
            .unwrap_or_else(|| panic!("{thread} linked"));
        assert!(!conflicted, "{thread}");
        assert_eq!(proof.launch_call_id, "call_launch");
        assert_eq!(proof.launch_operation_index, op);
        assert_eq!(proof.acknowledgment_operation_index, op);
        assert_eq!(proof.acknowledgment_call_id, answered[op as usize]);
        assert_eq!(proof.process_session_id.as_deref(), Some(handle));
        assert_eq!(proof.first_record_uuid, first);
        assert_eq!(
            shown_parent(&store, &child),
            Some((format!("codex-{PARENT}"), ParentEvidence::AgentLaunch))
        );
        assert_eq!(facts(&store, &child).len(), 1, "{thread}");
    }
    let third = session(&store, THIRD);
    assert!(facts(&store, &third).is_empty());
    assert!(store.session_creation(&third).unwrap().is_none());

    drop(store);
    let mut store = home.store();
    let again = settle(&home, &mut store, &mut SpawnBacklog::starting());
    assert_eq!(
        (again.files_read, again.changed, again.linked),
        (0, 0, 0),
        "{again:?}"
    );
    for thread in [CHILD, OTHER] {
        assert!(
            store
                .codex_cli_launch_creation(&session(&store, thread))
                .unwrap()
                .is_some()
        );
    }
}

fn unrelated(offset: i64) -> [Value; 2] {
    [
        call(offset, "call_unrelated", &cell(&["git status"])),
        output(
            offset + 50,
            "call_unrelated",
            DONE,
            &[exited(0, &started(THIRD))],
        ),
    ]
}

/// Both runs' first results arrive together in one wait on the cell.
#[test]
fn two_launches_answered_in_one_wait_each_link_their_own_child() {
    let mut body = vec![
        call(0, "call_launch", &cell(&[COMMAND, COMMAND])),
        output(1000, "call_launch", "Script running with cell ID 12\n", &[]),
    ];
    body.extend(unrelated(1100));
    body.extend([
        wait(1200, "call_wait", "12"),
        output(
            7319,
            "call_wait",
            DONE,
            &[running_as(70001, CHILD), running_as(70002, OTHER)],
        ),
    ]);
    two_launches_in_one_cell(body, ["call_wait", "call_wait"]);
}

/// The first run's result is already in the launch's own output; the wait
/// emits only the second's.
#[test]
fn a_launch_answered_before_the_wait_and_one_in_it_each_link_their_own_child() {
    let mut body = vec![
        call(0, "call_launch", &cell(&[COMMAND, COMMAND])),
        output(
            1000,
            "call_launch",
            "Script running with cell ID 12\n",
            &[running_as(70001, CHILD)],
        ),
    ];
    body.extend(unrelated(1100));
    body.extend([
        wait(1200, "call_wait", "12"),
        output(7319, "call_wait", DONE, &[running_as(70002, OTHER)]),
    ]);
    two_launches_in_one_cell(body, ["call_launch", "call_wait"]);
}
