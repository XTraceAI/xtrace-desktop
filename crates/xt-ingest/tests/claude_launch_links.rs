//! Automatic parent links for Claude sessions a Codex agent launched with an
//! explicit `--session-id`, through the ordinary passes.
//!
//! Every file is written here: a synthetic paginated Codex history (one
//! original rollout the index records and continuations it does not) holding
//! one launch in the shape a coordinator writes, and the launched Claude
//! session's transcript. Nothing real is read and no command runs.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use xt_ingest::native::{
    HostStatus, ImportRequest, ProducerSource,
    claude_launch::{LaunchBacklog, LaunchLimits, LaunchProgress, continue_claude_launches},
    import_reader_lines,
    readers_cli::{ReaderOutcome, read_pin},
    session_creation::{SpawnBacklog, continue_codex_spawns, spawn_limits},
    session_titles::TitleLimits,
    watch::{ProbePoint, TailEvent, Tailer, WatchConfig},
};
use xt_store::{
    Host, Store,
    claude_launch::ChildState,
    creation::{CreationEvidence, CreationWitness},
    session_list::{ParentEvidence, SessionFilter},
};

const T: i64 = 1_790_700_000_000;
const PARENT: &str = "01a00000-0000-7000-8000-0000000000aa";
const ROLLOUTS: [&str; 5] = [
    "01a00000-0000-7000-8000-0000000000b1",
    "01a00000-0000-7000-8000-0000000000b2",
    "01a00000-0000-7000-8000-0000000000b3",
    "01a00000-0000-7000-8000-0000000000b4",
    "01a00000-0000-7000-8000-0000000000b5",
];
const CHILD: &str = "0c000000-0000-4000-8000-0000000000c1";
const FIRST: &str = "0d000000-0000-4000-8000-0000000000f1";
const HANDLE: u64 = 56751;
const PROMPT: &str = "SYNTHETIC worker brief, it's \"quoted\"\nsecond line";
/// The launch call's time; everything else is an offset from it.
const AT: i64 = 1_790_668_716_333;

fn iso(offset: i64) -> String {
    chrono::DateTime::from_timestamp_millis(AT + offset)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn js(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bundle() -> ProducerSource {
    ProducerSource::Bundle {
        pin: read_pin(&repo().join(".plugin-pin")).unwrap(),
        root: repo().join("vendor/agent-plugins"),
    }
}

/// A synthetic home with an installed Claude launcher.
struct Home {
    _temp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    db: PathBuf,
}

impl Home {
    fn new() -> Self {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("home");
        let bin = home.join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("claude"), "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(home.join(".codex/sessions")).unwrap();
        fs::create_dir_all(home.join(".claude/projects")).unwrap();
        Self {
            db: root.join("index.sqlite"),
            _temp: temp,
            root,
            home,
        }
    }

    fn claude(&self) -> String {
        self.home.join(".local/bin/claude").display().to_string()
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

const DONE: &str = "Script completed\nOutput:\n";

fn live(handle: u64) -> Value {
    json!({"chunk_id": "a1", "wall_time_seconds": 10.0, "session_id": handle, "output": ""})
}

fn exit(code: i64, text: &str) -> Value {
    json!({"chunk_id": "d06517", "wall_time_seconds": 0.1, "exit_code": code, "output": text})
}

fn message(offset: i64, text: &str) -> Value {
    json!({"timestamp": iso(offset), "type": "response_item", "payload": {
        "type": "message", "role": "assistant",
        "content": [{"type": "output_text", "text": text}]}})
}

/// The launch cell as a coordinator writes it: a constant prompt, quoted by
/// `.replace`, every accepted option, and output redirected.
fn launch_cell(claude: &str, options: &str, child: &str) -> String {
    let prefix = format!(
        "{claude} -p --session-id {child} --safe-mode --model claude-opus-5-5 --effort high \
         --permission-mode bypassPermissions --tools Read,Grep --allowedTools Bash \
         --disable-slash-commands --strict-mcp-config --mcp-config '{{\"mcpServers\":{{}}}}'{options} \
         --output-format json '"
    );
    format!(
        "const p = {};\nconst r = await tools.exec_command({{cmd: {} + p.replace(/'/g, \"'\\\\''\") + {}, workdir: \"/w\", yield_time_ms: 10000}});\ntext(r)",
        js(PROMPT),
        js(&prefix),
        js("' > /w/result.json")
    )
}

fn check_cell() -> String {
    "text(await tools.exec_command({cmd: \"test -s /w/result.json\", workdir: \"/w\"}))".into()
}

fn poll_cell(handle: u64) -> String {
    format!(
        "text(await tools.write_stdin({{session_id: {handle}, chars: \"\", yield_time_ms: 30000}}))"
    )
}

/// The launch and its process as the actual history has them: a one-operation
/// launch returning a handle; poll cells whose first operation is an
/// unrelated check that exits 0 and whose second polls; one poll that needed
/// a wait; a completion cell whose first operation polls to exit 0 and whose
/// second is another command; then a later resume of the same child, which
/// is not a creation.
fn launch_rows(claude: &str, child: &str, completion: Value) -> Vec<Value> {
    let poll_then = |check: bool| {
        if check {
            format!("{};\n{}", check_cell(), poll_cell(HANDLE))
        } else {
            format!("{};\n{}", poll_cell(HANDLE), check_cell())
        }
    };
    vec![
        message(-2000, "Starting a synthetic worker"),
        call(0, "call_launch", &launch_cell(claude, "", child)),
        message(100, "between"),
        output(200, "call_launch", DONE, &[live(HANDLE)]),
        call(60_000, "call_poll_1", &poll_then(true)),
        output(90_000, "call_poll_1", DONE, &[exit(0, "ok"), live(HANDLE)]),
        call(300_000, "call_poll_2", &poll_then(true)),
        output(
            301_000,
            "call_poll_2",
            "Script running with cell ID 12\n",
            &[exit(0, "")],
        ),
        wait(302_000, "call_wait_2", "12"),
        output(330_000, "call_wait_2", DONE, &[live(HANDLE)]),
        call(945_000, "call_completion", &poll_then(false)),
        message(945_100, "unrelated"),
        output(
            945_197,
            "call_completion",
            DONE,
            &[completion, exit(0, "{}")],
        ),
        call(
            2_691_756,
            "call_resume",
            &format!(
                "text(await tools.exec_command({{cmd: {}, workdir: \"/w\"}}))",
                js(&format!(
                    "{claude} -p --resume {child} --output-format json 'Resumed request' > /w/r.json"
                ))
            ),
        ),
        output(2_691_900, "call_resume", DONE, &[live(57728)]),
    ]
}

fn write_rows(path: &Path, rows: &[Value]) -> u64 {
    let body: String = rows.iter().map(|row| format!("{row}\n")).collect();
    fs::write(path, &body).unwrap();
    body.len() as u64
}

/// Write a linear paginated history: the original, then one continuation
/// per extra body, each continuing the whole previous file. Returns the
/// files, the original first.
fn write_history(home: &Home, bodies: &[Vec<Value>]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut ordinal = 0_i64;
    let mut base: Option<Value> = None;
    for (index, body) in bodies.iter().enumerate() {
        let (path, rollout) = if index == 0 {
            (
                home.day("2026/09/07")
                    .join(format!("rollout-2026-09-07T19-04-03-{PARENT}.jsonl")),
                PARENT,
            )
        } else {
            let rollout = ROLLOUTS[index - 1];
            (
                home.day(&format!("2026/09/{}", 10 + 6 * index))
                    .join(format!(
                        "rollout-2026-09-{}T16-14-04-{PARENT}_{rollout}.jsonl",
                        10 + 6 * index
                    )),
                rollout,
            )
        };
        let mut rows = vec![
            json!({"timestamp": iso(-86_400_000 + index as i64), "type": "session_meta",
            "ordinal": ordinal, "payload": {"id": PARENT, "originator": "codex_cli_rs",
            "cwd": "/w", "source": "cli", "history_mode": "paginated",
            "history_base": base.clone().unwrap_or(Value::Null)}}),
        ];
        ordinal += 1;
        for row in body {
            let mut row = row.clone();
            row["ordinal"] = json!(ordinal);
            ordinal += 1;
            rows.push(row);
        }
        let length = write_rows(&path, &rows);
        base = Some(json!({"thread_id": rollout, "end_byte_offset": length,
            "end_ordinal_exclusive": ordinal}));
        files.push(path);
    }
    files
}

/// The actual-shaped parent: an indexed original and three continuations,
/// the launch in the last.
fn actual_history(home: &Home, completion: Value) -> Vec<PathBuf> {
    let filler = |n: i64| {
        (0..n)
            .map(|i| message(-3_000_000 + i, "Earlier synthetic work"))
            .collect::<Vec<_>>()
    };
    write_history(
        home,
        &[
            filler(20),
            filler(10),
            filler(5),
            launch_rows(&home.claude(), CHILD, completion),
        ],
    )
}

/// Index the parent exactly as the reader does: only its original rollout.
fn index_parent(store: &mut Store, original: &Path) {
    let lines = vec![
        json!({"type": "session", "host": "codex", "native_session_id": PARENT,
               "conversation_id": format!("codex-{PARENT}"), "source_surface": "codex_cli",
               "started_at": iso(-86_400_000), "cwd": "/w", "git_branch": null, "title": null,
               "path": original.display().to_string(), "mtime": 1790668716.0})
        .to_string(),
        json!({"uuid": "5555aaaa-5555-4555-8555-000000000000", "type": "user", "cwd": "/w",
               "timestamp": iso(-2500),
               "message": {"role": "user", "content": [{"type": "text", "text": "Synthetic request"}]}})
        .to_string(),
    ];
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

fn child_rows(first_offset: i64, text: &str) -> Vec<Value> {
    let line = |uuid: &str, offset: i64, content: Value| {
        json!({"type": "user", "uuid": uuid, "parentUuid": null, "timestamp": iso(offset),
            "sessionId": CHILD, "cwd": "/w", "entrypoint": "sdk-cli", "promptSource": "sdk",
            "message": {"role": "user", "content": content}})
    };
    vec![
        json!({"type": "queue-operation", "operation": "enqueue", "timestamp": iso(first_offset - 1000),
            "sessionId": CHILD}),
        line(FIRST, first_offset, json!(text)),
        json!({"type": "assistant", "uuid": "0d000000-0000-4000-8000-0000000000a1",
            "parentUuid": FIRST, "timestamp": iso(first_offset + 5000), "sessionId": CHILD,
            "cwd": "/w", "entrypoint": "sdk-cli",
            "message": {"role": "assistant", "id": "msg_synthetic", "model": "fixture-model",
                "content": [{"type": "text", "text": "Synthetic answer"}],
                "usage": {"input_tokens": 3, "output_tokens": 2}}}),
        line(
            "0d000000-0000-4000-8000-0000000000d2",
            2_691_800 + 2000,
            json!("Resumed request"),
        ),
    ]
}

fn write_child(home: &Home, rows: &[Value]) -> PathBuf {
    let project = home.home.join(".claude/projects/-w");
    fs::create_dir_all(&project).unwrap();
    let path = project.join(format!("{CHILD}.jsonl"));
    write_rows(&path, rows);
    path
}

/// Index the Claude host through the ordinary importer, queueing what it
/// imported into `spawns`, as a watcher's scan does.
fn scan_claude(home: &Home, store: &mut Store, spawns: &mut SpawnBacklog) {
    let report = xt_ingest::native::scan_native_continued(
        store,
        &ImportRequest {
            home: &home.home,
            hosts: &[Host::Claude],
            producer: &bundle(),
            python: None,
            observed_at: T,
            cancel: None,
        },
        xt_ingest::native::ScanMode::Resume,
        &mut |_| {},
        spawns,
        spawn_limits(),
    );
    assert!(report.complete(), "{report:?}");
}

fn child_session(store: &Store) -> Option<String> {
    store
        .user_sessions_with_native(Host::Claude, CHILD)
        .unwrap()
        .into_iter()
        .next()
}

/// Run the ordinary watcher passes until nothing moves. Returns every pass.
fn settle(home: &Home, store: &mut Store, spawns: &mut SpawnBacklog) -> Vec<LaunchProgress> {
    let mut passes = Vec::new();
    for _ in 0..200 {
        let progress =
            continue_codex_spawns(store, &home.home, spawns, spawn_limits(), None, T).unwrap();
        let launches = progress.launches.clone();
        let moved = progress.advanced();
        if let Some(launches) = launches {
            passes.push(launches);
        }
        if !moved {
            break;
        }
    }
    passes
}

fn totals(passes: &[LaunchProgress]) -> LaunchProgress {
    let mut total = LaunchProgress::default();
    for pass in passes {
        total.files_read += pass.files_read;
        total.files_reused += pass.files_reused;
        total.threads_unchanged += pass.threads_unchanged;
        total.launches_found += pass.launches_found;
        total.launches_broken += pass.launches_broken;
        total.linked += pass.linked;
        total.changed += pass.changed;
        total.rejected += pass.rejected;
        total.waiting += pass.waiting;
        total.bytes_read += pass.bytes_read;
        total.published_invalid += pass.published_invalid;
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

/// Parent indexed, child indexed, then one ordinary start: the launch in the
/// latest unindexed continuation is linked, the resume is not a second
/// creation, a restart changes nothing and reads no history again, and the
/// child's user messages now count as the agent's in the app's hours.
#[test]
fn links_the_actual_shaped_launch_from_an_unindexed_continuation_and_stays_linked() {
    let home = Home::new();
    let files = actual_history(&home, exit(0, ""));
    write_child(&home, &child_rows(4203, PROMPT));
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let child = child_session(&store).expect("child indexed");
    let before = eligible(&home.db, &child);
    assert!(before >= 1, "the child's inputs counted as typed before");
    let parent_before = eligible(&home.db, &format!("codex-{PARENT}"));

    let mut spawns = SpawnBacklog::starting();
    let passes = settle(&home, &mut store, &mut spawns);
    let total = totals(&passes);
    assert_eq!(total.linked, 1, "{total:?}");
    assert_eq!(total.changed, 1, "{total:?}");
    assert_eq!(total.launches_found, 1, "{total:?}");
    assert_eq!(total.files_read, 4, "{total:?}");
    let history: u64 = files.iter().map(|f| fs::metadata(f).unwrap().len()).sum();
    assert!(total.bytes_read >= history, "every file read whole once");

    let (proof, conflicted) = store
        .claude_launch_creation(&child)
        .unwrap()
        .expect("linked");
    assert!(!conflicted);
    assert_eq!(proof.parent_session_id, format!("codex-{PARENT}"));
    assert_eq!(proof.parent_native_session_id, PARENT);
    assert_eq!(proof.first_record_uuid, FIRST);
    assert_eq!(proof.launch_call_id, "call_launch");
    assert_eq!(proof.launch_operation_index, 0);
    // Acknowledged by the launch's own reply: its running process.
    assert_eq!(proof.acknowledgment_call_id, "call_launch");
    assert_eq!(proof.acknowledgment_operation_index, 0);
    assert_eq!(proof.process_session_id.as_deref(), Some("56751"));
    assert_eq!(proof.segment_rollout_id, ROLLOUTS[2]);
    assert_eq!(
        proof.acknowledgment_ordinal.unwrap(),
        proof.launch_ordinal.unwrap() + 2
    );
    assert_eq!(proof.evidence_version, 3);
    let stored = store.session_creation(&child).unwrap().unwrap().0;
    assert_eq!(stored.evidence_kind, CreationEvidence::CodexClaudeLaunch);
    assert_eq!(stored.witness, CreationWitness::CodexExecSessionIdLaunch);
    assert_eq!(
        shown_parent(&store, &child),
        Some((format!("codex-{PARENT}"), ParentEvidence::AgentLaunch))
    );
    // The existing rule: an agent-created session's user messages are not
    // typed by a person. Only the new child changes.
    assert_eq!(eligible(&home.db, &child), 0);
    assert_eq!(
        eligible(&home.db, &format!("codex-{PARENT}")),
        parent_before
    );
    let summary = store.claude_launch_summary().unwrap();
    assert_eq!(summary.relations_accepted, 1);
    assert_eq!(summary.candidates_linked, 1);
    assert_eq!((summary.members, summary.groups_valid), (4, 1));

    // A restart: nothing changes, no history file is read again.
    drop(store);
    let mut store = home.store();
    let mut spawns = SpawnBacklog::starting();
    let again = totals(&settle(&home, &mut store, &mut spawns));
    assert_eq!(
        (again.files_read, again.changed, again.linked),
        (0, 0, 0),
        "{again:?}"
    );
    assert_eq!(
        (again.threads_unchanged, again.bytes_read),
        (1, 0),
        "{again:?}"
    );
    assert!(store.claude_launch_creation(&child).unwrap().is_some());
}

/// Parent first, while the launched job still runs and is never seen to
/// end: the launch is found from its own reply and waits for its child
/// without busy retries; the child's import links it on the next pass,
/// reading no history again. The job's later exit changes nothing.
#[test]
fn parent_first_waits_for_the_child_of_a_job_still_running() {
    let home = Home::new();
    // The process is still running at the end of the history.
    let files = actual_history(&home, live(HANDLE));
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    let mut spawns = SpawnBacklog::starting();
    let found = totals(&settle(&home, &mut store, &mut spawns));
    assert_eq!((found.launches_found, found.linked), (1, 0), "{found:?}");
    assert_eq!(found.waiting, 1);
    assert!(
        !spawns.launches.pending(),
        "a missing child is not retried on a timer"
    );
    // The child is imported: its launch is linked on the next pass.
    write_child(&home, &child_rows(4203, PROMPT));
    scan_claude(&home, &mut store, &mut spawns);
    let linked = totals(&settle(&home, &mut store, &mut spawns));
    assert_eq!(linked.linked, 1, "{linked:?}");
    assert_eq!(linked.files_read, 0, "no history is read again for a child");
    let child = child_session(&store).unwrap();
    let before = store.claude_launch_creation(&child).unwrap().unwrap();
    // The job's exit is appended: the history is read again, the launch is
    // the same, and the relation stays exactly as it was.
    let last = files.last().unwrap();
    let body = fs::read_to_string(last).unwrap();
    let completion = body
        .lines()
        .find(|line| {
            line.contains("\"call_completion\"") && line.contains("custom_tool_call_output")
        })
        .unwrap()
        .to_owned();
    let fixed = completion.replace(
        &serde_json::to_string(&live(HANDLE).to_string()).unwrap(),
        &serde_json::to_string(&exit(1, "").to_string()).unwrap(),
    );
    assert_ne!(fixed, completion);
    fs::write(last, body.replace(&completion, &fixed)).unwrap();
    spawns.launches.add_threads([PARENT]);
    let after = totals(&settle(&home, &mut store, &mut spawns));
    assert!(after.files_read >= 1, "{after:?}");
    assert_eq!((after.linked, after.changed), (0, 0), "{after:?}");
    assert_eq!(
        store.claude_launch_creation(&child).unwrap().unwrap(),
        before
    );
}

/// The launch's own reply rewritten in place so it is no longer a process
/// result, without changing the file's length, after the launch was found
/// and before the child arrives: the file is read again from its first byte,
/// the launch is no longer acknowledged, and nothing is linked.
#[test]
fn a_same_length_rewrite_before_the_child_arrives_links_nothing() {
    let home = Home::new();
    let files = actual_history(&home, exit(0, ""));
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    let mut spawns = SpawnBacklog::starting();
    assert_eq!(
        totals(&settle(&home, &mut store, &mut spawns)).launches_found,
        1
    );
    let last = files.last().unwrap();
    let before = fs::metadata(last).unwrap().len();
    let body = fs::read_to_string(last).unwrap();
    let rewritten = body.replacen("\\\"session_id\\\":56751", "\\\"session_zz\\\":56751", 1);
    assert_ne!(rewritten, body);
    fs::write(last, &rewritten).unwrap();
    assert_eq!(fs::metadata(last).unwrap().len(), before, "same length");
    write_child(&home, &child_rows(4203, PROMPT));
    scan_claude(&home, &mut store, &mut spawns);
    let total = totals(&settle(&home, &mut store, &mut spawns));
    assert_eq!(total.linked, 0, "{total:?}");
    assert!(total.files_read >= 1, "the changed file was read again");
    let child = child_session(&store).unwrap();
    assert!(store.session_creation(&child).unwrap().is_none());
    // The rewritten launch is not acknowledged: the new validation holds no
    // launch for the child, and nothing was linked.
    let rows = store
        .claude_launch_candidates_for_child(CHILD, None, 10)
        .unwrap();
    assert!(
        rows.iter().all(|row| row.child_state != ChildState::Linked),
        "{rows:?}"
    );
    assert!(rows.is_empty(), "{rows:?}");
}

/// Each variant of the actual history's launch or its own reply relates
/// nothing; what happens after that reply — another operation's exit, a
/// missing or extra later result, input sent, the handle handed out
/// elsewhere — changes nothing.
#[test]
fn launches_outside_the_supported_form_or_unacknowledged_relate_nothing() {
    type Edit = Box<dyn Fn(&Home, &mut Vec<Value>)>;
    let replace_cell = |from: &'static str, to: &'static str| -> Edit {
        Box::new(move |_, rows: &mut Vec<Value>| {
            let input = rows[1]["payload"]["input"]
                .as_str()
                .unwrap()
                .replace(from, to);
            assert_ne!(
                input,
                rows[1]["payload"]["input"].as_str().unwrap(),
                "{from}"
            );
            rows[1]["payload"]["input"] = json!(input);
        })
    };
    let cases: Vec<(&str, Edit)> = vec![
        // The same setup unedited links: every refusal below is its edit's.
        ("control", Box::new(|_, _| {})),
        ("resume", replace_cell("-p --session-id", "-p --resume")),
        (
            "fork",
            replace_cell("--safe-mode", "--safe-mode --fork-session"),
        ),
        (
            "continue",
            replace_cell("--safe-mode", "--safe-mode --continue"),
        ),
        ("help", replace_cell("--safe-mode", "--safe-mode --help")),
        (
            "unknown option",
            replace_cell("--safe-mode", "--safe-mode --agent x"),
        ),
        (
            "stdin prompt",
            replace_cell("' > /w/result.json", "' < /w/brief.md"),
        ),
        ("pipe", replace_cell("' > /w/result.json", "' | tee /w/o")),
        (
            "template",
            replace_cell("const p = ", "const q = `x`; const p = "),
        ),
        (
            "bare program",
            Box::new(|home: &Home, rows: &mut Vec<Value>| {
                let input = rows[1]["payload"]["input"]
                    .as_str()
                    .unwrap()
                    .replace(&home.claude(), "claude");
                rows[1]["payload"]["input"] = json!(input);
            }),
        ),
        (
            "other executable",
            Box::new(|home: &Home, rows: &mut Vec<Value>| {
                let input = rows[1]["payload"]["input"]
                    .as_str()
                    .unwrap()
                    .replace(&home.claude(), "/tmp/elsewhere/claude");
                rows[1]["payload"]["input"] = json!(input);
            }),
        ),
        // The launch's own reply: not a process result, missing, extra,
        // another call's, or absent.
        (
            "tool error reply",
            Box::new(|_, rows: &mut Vec<Value>| {
                rows[3] = raw_output(200, "call_launch", DONE, &["exec_command failed"]);
            }),
        ),
        (
            "missing reply result",
            Box::new(|_, rows: &mut Vec<Value>| {
                rows[3] = output(200, "call_launch", DONE, &[]);
            }),
        ),
        (
            "extra reply result",
            Box::new(|_, rows: &mut Vec<Value>| {
                rows[3] = output(200, "call_launch", DONE, &[live(HANDLE), exit(0, "")]);
            }),
        ),
        (
            "reply to another call",
            Box::new(|_, rows: &mut Vec<Value>| {
                rows[3] = output(200, "call_other", DONE, &[live(HANDLE)]);
            }),
        ),
        (
            "no reply",
            Box::new(|_, rows: &mut Vec<Value>| {
                rows.remove(3);
            }),
        ),
        // After the reply: the completion cell's other operation exits 0
        // but the process 1.
        (
            "later: wrong operation exit",
            Box::new(|_, rows: &mut Vec<Value>| {
                rows[12] = output(
                    945_197,
                    "call_completion",
                    DONE,
                    &[exit(1, ""), exit(0, "{}")],
                );
            }),
        ),
        (
            "later: missing result",
            Box::new(|_, rows: &mut Vec<Value>| {
                rows[12] = output(945_197, "call_completion", DONE, &[exit(0, "")]);
            }),
        ),
        (
            "later: extra result",
            Box::new(|_, rows: &mut Vec<Value>| {
                rows[12] = output(
                    945_197,
                    "call_completion",
                    DONE,
                    &[exit(0, ""), exit(0, ""), exit(0, "")],
                );
            }),
        ),
        (
            "later: input sent",
            Box::new(|_, rows: &mut Vec<Value>| {
                let input = rows[10]["payload"]["input"].as_str().unwrap().replacen(
                    "chars: \"\"",
                    "chars: \"y\"",
                    1,
                );
                rows[10]["payload"]["input"] = json!(input);
            }),
        ),
        (
            "later: handle handed out elsewhere",
            Box::new(|_, rows: &mut Vec<Value>| {
                rows.insert(7, output(200_000, "call_other", DONE, &[live(HANDLE)]));
            }),
        ),
        (
            "duplicate launch output",
            Box::new(|_, rows: &mut Vec<Value>| {
                let copy = rows[3].clone();
                rows.push(copy);
            }),
        ),
        (
            "duplicate launch call",
            Box::new(|_, rows: &mut Vec<Value>| {
                let copy = rows[1].clone();
                rows.insert(0, copy);
            }),
        ),
        // Mentions alone: in prose, in a quoted script, in an output.
        (
            "prose and quotes only",
            Box::new(|home: &Home, rows: &mut Vec<Value>| {
                let command = format!(
                    "{} -p --session-id {CHILD} --output-format json 'x'",
                    home.claude()
                );
                *rows = vec![
                    message(0, &command),
                    call(
                        10,
                        "call_quoted",
                        &format!(
                            "text(await tools.exec_command({{cmd: {}, workdir: \"/w\"}}))",
                            js(&format!("echo {}", command.replace(' ', "_")))
                        ),
                    ),
                    output(20, "call_quoted", DONE, &[exit(0, &command)]),
                ];
            }),
        ),
        (
            "second header mid-file",
            Box::new(|_, rows: &mut Vec<Value>| {
                rows.insert(5, json!({"timestamp": iso(1000), "type": "session_meta", "payload": {"id": PARENT}}));
            }),
        ),
    ];
    for (name, edit) in cases {
        let home = Home::new();
        let mut rows = launch_rows(&home.claude(), CHILD, exit(0, ""));
        edit(&home, &mut rows);
        let files = write_history(&home, &[vec![message(-3_000_000, "earlier")], rows]);
        write_child(&home, &child_rows(4203, PROMPT));
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
        scan_claude(&home, &mut store, &mut SpawnBacklog::default());
        let mut spawns = SpawnBacklog::starting();
        let total = totals(&settle(&home, &mut store, &mut spawns));
        let child = child_session(&store).unwrap();
        let linked = store.session_creation(&child).unwrap().is_some();
        assert_eq!(
            linked,
            name == "control" || name.starts_with("later: "),
            "{name}: {total:?}"
        );
    }
}

/// The child's side must match: another first input, one dated before the
/// launch, or an earlier image input links nothing. A first input long after
/// the job's exit still links: nothing in the parent's history bounds it.
#[test]
fn a_child_whose_first_input_is_not_the_launch_is_not_linked() {
    let cases: Vec<(&str, Vec<Value>)> = vec![
        ("control", child_rows(4203, PROMPT)),
        ("other text", child_rows(4203, "Something else")),
        ("before the launch", child_rows(-500, PROMPT)),
        ("long after the exit", child_rows(2_000_000, PROMPT)),
        ("image first", {
            let mut rows = child_rows(4203, PROMPT);
            rows.insert(
                1,
                json!({"type": "user", "uuid": "0d000000-0000-4000-8000-0000000000e1",
                    "parentUuid": null, "timestamp": iso(3000), "sessionId": CHILD, "cwd": "/w",
                    "message": {"role": "user", "content": [{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AA=="}}]}}),
            );
            rows
        }),
    ];
    for (name, rows) in cases {
        let home = Home::new();
        let files = actual_history(&home, exit(0, ""));
        write_child(&home, &rows);
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
        scan_claude(&home, &mut store, &mut SpawnBacklog::default());
        let mut spawns = SpawnBacklog::starting();
        let total = totals(&settle(&home, &mut store, &mut spawns));
        assert_eq!(
            total.linked,
            usize::from(name == "control" || name == "long after the exit"),
            "{name}: {total:?}"
        );
    }
}

/// A history that is not one valid group relates nothing: a broken base
/// cutoff, a continuation whose ordinals skip, a fork, and a spawned thread
/// whose launch lies in its inherited context.
#[test]
fn an_invalid_or_inherited_history_relates_nothing() {
    let rewrite_header = |edit: &dyn Fn(&mut Value)| {
        let home = Home::new();
        let files = actual_history(&home, exit(0, ""));
        let last = files.last().unwrap();
        let body = fs::read_to_string(last).unwrap();
        let (header, rest) = body.split_once('\n').unwrap();
        let mut header: Value = serde_json::from_str(header).unwrap();
        edit(&mut header);
        fs::write(last, format!("{header}\n{rest}")).unwrap();
        (home, files)
    };
    let cases: Vec<(&str, (Home, Vec<PathBuf>))> = vec![
        ("control", rewrite_header(&|_| {})),
        (
            "cutoff not a boundary",
            rewrite_header(&|h| {
                let end = h["payload"]["history_base"]["end_byte_offset"]
                    .as_u64()
                    .unwrap();
                h["payload"]["history_base"]["end_byte_offset"] = json!(end - 3);
            }),
        ),
        (
            "ordinal off by one",
            rewrite_header(&|h| {
                let ordinal = h["ordinal"].as_i64().unwrap();
                h["ordinal"] = json!(ordinal + 1);
            }),
        ),
        (
            "base outside the group",
            rewrite_header(&|h| {
                h["payload"]["history_base"]["thread_id"] =
                    json!("01a00000-0000-7000-8000-0000000000ff");
            }),
        ),
        (
            "fork",
            rewrite_header(&|h| {
                h["payload"]["forked_from_id"] = json!("01a00000-0000-7000-8000-0000000000ee");
            }),
        ),
        (
            "inherited context",
            rewrite_header(&|h| {
                // Everything this file holds is below the inherited boundary.
                h["payload"]["subagent_history_start_ordinal"] = json!(1_000_000);
            }),
        ),
    ];
    for (name, (home, files)) in cases {
        write_child(&home, &child_rows(4203, PROMPT));
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
        scan_claude(&home, &mut store, &mut SpawnBacklog::default());
        let mut spawns = SpawnBacklog::starting();
        let total = totals(&settle(&home, &mut store, &mut spawns));
        assert_eq!(
            total.linked,
            usize::from(name == "control"),
            "{name}: {total:?}"
        );
    }
    // An indexed locator the census does not find (an alias) refuses too.
    let home = Home::new();
    let files = actual_history(&home, exit(0, ""));
    write_child(&home, &child_rows(4203, PROMPT));
    let alias = home.root.join("alias.jsonl");
    std::os::unix::fs::symlink(&files[0], &alias).unwrap();
    let mut store = home.store();
    index_parent(&mut store, &alias);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let total = totals(&settle(&home, &mut store, &mut SpawnBacklog::starting()));
    assert_eq!(total.linked, 0, "{total:?}");
}

/// Many launches in one large history, read in several budget-bounded
/// passes, one of them after a restart: every child is linked, each file is
/// read whole about once per generation, and a second start reads nothing.
#[test]
fn a_large_history_is_read_in_bounded_passes_and_resumes_after_a_restart() {
    const CHILDREN: usize = 40;
    let home = Home::new();
    let child = |index: usize| format!("0c000000-0000-4000-8000-{:012x}", 0x100 + index);
    let mut bodies = Vec::new();
    for part in 0..4 {
        let mut rows = Vec::new();
        for index in (part * CHILDREN / 4)..((part + 1) * CHILDREN / 4) {
            let base = index as i64 * 10_000;
            let launch = format!("call_launch_{index}");
            rows.push(call(
                base,
                &launch,
                &launch_cell(&home.claude(), "", &child(index)),
            ));
            rows.push(output(base + 5, &launch, DONE, &[exit(0, "")]));
            for filler in 0..50 {
                rows.push(message(base + 10 + filler, &"filler ".repeat(40)));
            }
        }
        bodies.push(rows);
    }
    let files = write_history(&home, &bodies);
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    // Each child: its first input is the prompt, just after its launch.
    let project = home.home.join(".claude/projects/-w");
    fs::create_dir_all(&project).unwrap();
    for index in 0..CHILDREN {
        let native = child(index);
        let at = index as i64 * 10_000 + 2;
        let rows = vec![
            json!({"type": "user", "uuid": format!("0d000000-0000-4000-8000-{:012x}", 0x100 + index),
            "parentUuid": null, "timestamp": iso(at), "sessionId": native, "cwd": "/w",
            "message": {"role": "user", "content": PROMPT}}),
        ];
        write_rows(&project.join(format!("{native}.jsonl")), &rows);
    }
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let history: u64 = files.iter().map(|f| fs::metadata(f).unwrap().len()).sum();
    let limits = TitleLimits {
        max_batch_bytes: history / 3,
        ..spawn_limits()
    };
    let pass = |store: &mut Store, spawns: &mut SpawnBacklog| {
        continue_codex_spawns(store, &home.home, spawns, limits, None, T)
            .unwrap()
            .launches
            .unwrap_or_default()
    };
    let mut spawns = SpawnBacklog::starting();
    let mut read = 0;
    let mut passes = 0;
    // Two passes, then a restart with a fresh backlog.
    for _ in 0..2 {
        let progress = pass(&mut store, &mut spawns);
        read += progress.bytes_read;
        passes += 1;
    }
    drop(store);
    let mut store = home.store();
    let mut spawns = SpawnBacklog::starting();
    let mut linked = 0;
    for _ in 0..100 {
        let progress = pass(&mut store, &mut spawns);
        read += progress.bytes_read;
        linked += progress.linked;
        passes += 1;
        if !spawns.launches.pending() {
            break;
        }
    }
    let summary = store.claude_launch_summary().unwrap();
    assert_eq!(summary.relations_accepted, CHILDREN as u64, "{summary:?}");
    assert!(passes >= 3, "the budget split the work: {passes}");
    assert!(linked <= CHILDREN);
    // Files are read whole once per generation (plus headers and the two
    // lines per launch), not once per child.
    assert!(read < history * 2 + 4 * 1024 * 1024, "{read} vs {history}");
    println!("history_bytes={history} bytes_read={read} passes={passes}");
    // A later start reads no history.
    let mut backlog = LaunchBacklog::starting();
    let mut again = LaunchProgress::default();
    for _ in 0..10 {
        let progress = continue_claude_launches(
            &mut store,
            &home.home,
            &mut backlog,
            &LaunchLimits::default(),
            None,
            T,
        )
        .unwrap();
        again.files_read += progress.files_read;
        again.changed += progress.changed;
        if !backlog.pending() {
            break;
        }
    }
    assert_eq!((again.files_read, again.changed), (0, 0));
}

/// The ordinary watcher: the parent is indexed and the child's transcript is
/// on disk before it starts; with no source event after its start and the
/// Codex reader unavailable, its own passes link the child, announce it
/// once, and write no host file.
#[test]
fn the_watcher_links_a_saved_launch_without_a_new_event() {
    let home = Home::new();
    let files = actual_history(&home, exit(0, ""));
    write_child(&home, &child_rows(4203, PROMPT));
    {
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
    }
    let snapshot = |root: &Path| {
        let mut all = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    all.push((path.clone(), fs::read(&path).unwrap()));
                }
            }
        }
        all.sort();
        all
    };
    let before = snapshot(&home.home);
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let events = Arc::clone(&events);
        Box::new(move |event: TailEvent| events.lock().unwrap().push(event))
    };
    let passes = Arc::new(Mutex::new(Vec::new()));
    let probe = {
        let passes = Arc::clone(&passes);
        Arc::new(move |point: ProbePoint<'_>| {
            if let ProbePoint::SpawnsContinued { pending } = point {
                passes.lock().unwrap().push(pending);
            }
        })
    };
    let missing = home.root.join("nonexistent-python");
    let tailer = Tailer::start(
        home.store(),
        WatchConfig {
            home: home.home.clone(),
            hosts: vec![Host::Codex, Host::Claude],
            producer: bundle(),
            python: Some(missing.into_os_string()),
            debounce: Duration::from_millis(50),
            spawn_limits: spawn_limits(),
            probe: Some(probe),
        },
        sink,
    );
    assert!(tailer.wait_ready(Duration::from_secs(60)).is_some());
    let reader = home.store();
    let started = Instant::now();
    loop {
        if let Some(child) = child_session(&reader)
            && reader.claude_launch_creation(&child).unwrap().is_some()
        {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(60), "not linked");
        std::thread::sleep(Duration::from_millis(50));
    }
    // Let the worker go quiet.
    let started = Instant::now();
    while passes.lock().unwrap().last() != Some(&false) {
        assert!(started.elapsed() < Duration::from_secs(60), "never quiet");
        std::thread::sleep(Duration::from_millis(20));
    }
    tailer.stop();
    let announced: Vec<usize> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            TailEvent::SessionCreationsChanged { changed } => Some(*changed),
            _ => None,
        })
        .collect();
    assert_eq!(announced.iter().sum::<usize>(), 1, "{announced:?}");
    assert_eq!(snapshot(&home.home), before, "no host file written");
    let _ = LaunchLimits::default();
}

/// Run the production launch drainer directly with `limits` until nothing
/// is pending or 10 000 passes ran. Returns every pass.
fn drain(
    home: &Home,
    store: &mut Store,
    backlog: &mut LaunchBacklog,
    limits: &LaunchLimits,
) -> Vec<LaunchProgress> {
    let mut passes = Vec::new();
    for _ in 0..10_000 {
        let progress =
            continue_claude_launches(store, &home.home, backlog, limits, None, T).unwrap();
        passes.push(progress);
        if !backlog.pending() {
            break;
        }
    }
    passes
}

fn tiny(bytes: u64) -> LaunchLimits {
    LaunchLimits {
        max_pass_bytes: bytes,
        ..LaunchLimits::default()
    }
}

/// A history large beside the pass budget: the launch sits after ~1 MB of
/// rows in the last file.
fn large_history(home: &Home, rows_per_file: i64) -> Vec<PathBuf> {
    let filler = |n: i64, base: i64| {
        (0..n)
            .map(|i| message(base + i, &"synthetic filler ".repeat(60)))
            .collect::<Vec<_>>()
    };
    let mut last = filler(rows_per_file, -3_000_000);
    last.extend(launch_rows(&home.claude(), CHILD, exit(0, "")));
    write_history(
        home,
        &[
            filler(rows_per_file, -3_600_000),
            filler(rows_per_file, -3_400_000),
            filler(rows_per_file, -3_200_000),
            last,
        ],
    )
}

fn history_bytes(files: &[PathBuf]) -> u64 {
    files.iter().map(|f| fs::metadata(f).unwrap().len()).sum()
}

/// With a 64 KiB pass budget over ~4 MB of history, every pass stays within
/// its budget (plus at most the history's opening lines when a thread is
/// first planned), the passes together read each byte about once — a pass
/// that ends resumes where it stopped rather than from the start — and the
/// launch is linked.
#[test]
fn a_tiny_budget_spreads_one_read_over_many_passes_without_rereading() {
    let home = Home::new();
    let files = large_history(&home, 1_000);
    write_child(&home, &child_rows(4203, PROMPT));
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let history = history_bytes(&files);
    let budget = 64 * 1024;
    let mut backlog = LaunchBacklog::starting();
    let passes = drain(&home, &mut store, &mut backlog, &tiny(budget));
    let per_pass: Vec<u64> = passes.iter().map(|pass| pass.bytes_read).collect();
    let total: u64 = per_pass.iter().sum();
    let linked: usize = passes.iter().map(|pass| pass.linked).sum();
    println!(
        "history_bytes={history} passes={} bytes_per_pass_max={} total_bytes={total}",
        per_pass.len(),
        per_pass.iter().max().unwrap()
    );
    assert_eq!(linked, 1);
    assert!(
        per_pass.len() as u64 >= history / budget,
        "{} passes",
        per_pass.len()
    );
    // The opening lines of four files are read when the thread is planned.
    let headers: u64 = files
        .iter()
        .map(|f| fs::read_to_string(f).unwrap().lines().next().unwrap().len() as u64 + 1)
        .sum();
    assert!(
        per_pass
            .iter()
            .all(|bytes| *bytes <= budget + headers + 8 * 1024),
        "{per_pass:?}"
    );
    // Each byte about once: no pass started a file again.
    assert!(total < history + history / 10, "{total} vs {history}");
    assert!(total >= history);
}

/// A restart during a validation: nothing partial was stored, so the thread
/// is validated again from every file's first byte (the accepted tradeoff),
/// then linked; after that, a restart reads no history at all.
#[test]
fn a_restart_revalidates_an_unfinished_history_and_skips_a_validated_one() {
    let home = Home::new();
    let files = large_history(&home, 1_000);
    write_child(&home, &child_rows(4203, PROMPT));
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let budget = 128 * 1024;
    let mut backlog = LaunchBacklog::starting();
    // Run until two files were read, the third under way.
    let mut before = 0;
    let mut read = 0;
    for _ in 0..10_000 {
        let progress =
            continue_claude_launches(&mut store, &home.home, &mut backlog, &tiny(budget), None, T)
                .unwrap();
        before += progress.bytes_read;
        read += progress.files_read;
        if read == 2 {
            break;
        }
    }
    let summary = store.claude_launch_summary().unwrap();
    assert_eq!(
        (
            summary.groups_pending,
            summary.members,
            summary.relations_accepted
        ),
        (1, 0, 0),
        "an unfinished validation is pending and publishes nothing"
    );
    // Restart: a fresh backlog and store handle.
    drop(store);
    let mut store = home.store();
    let mut backlog = LaunchBacklog::starting();
    let passes = drain(&home, &mut store, &mut backlog, &tiny(budget));
    let after: u64 = passes.iter().map(|pass| pass.bytes_read).sum();
    let read: usize = passes.iter().map(|pass| pass.files_read).sum();
    let history = history_bytes(&files);
    println!("before_restart={before} after_restart={after} history={history}");

    assert_eq!(read, 4, "every file is read again");
    assert!(
        after >= history && after < history + history / 10,
        "{after} vs {history}"
    );
    assert!(passes.iter().all(|pass| pass.bytes_read <= budget));
    assert_eq!(passes.iter().map(|pass| pass.linked).sum::<usize>(), 1);
    // A validated history is not read after the next restart.
    drop(store);
    let mut store = home.store();
    let mut backlog = LaunchBacklog::starting();
    let again = totals(&drain(&home, &mut store, &mut backlog, &tiny(budget)));
    assert_eq!(
        (again.files_read, again.bytes_read, again.threads_unchanged),
        (0, 0, 1)
    );
}

/// A member that changes while it is read — appended to, or rewritten in
/// place with the same length — is read again from its first byte; the
/// members already read are kept in memory and not read again, and the
/// launch is linked from a validation the files still match.
#[test]
fn a_member_changed_during_its_read_is_read_again_alone() {
    for rewrite in [false, true] {
        let home = Home::new();
        let files = large_history(&home, 400);
        write_child(&home, &child_rows(4203, PROMPT));
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
        scan_claude(&home, &mut store, &mut SpawnBacklog::default());
        let last = files.last().unwrap().clone();
        let mut backlog = LaunchBacklog::starting();
        let limits = tiny(64 * 1024);
        let mut total = 0;
        // Run until three files were read, then one pass into the last.
        let mut read = 0;
        for _ in 0..10_000 {
            let progress =
                continue_claude_launches(&mut store, &home.home, &mut backlog, &limits, None, T)
                    .unwrap();
            total += progress.bytes_read;
            read += progress.files_read;
            if read == 3 {
                break;
            }
        }
        let progress =
            continue_claude_launches(&mut store, &home.home, &mut backlog, &limits, None, T)
                .unwrap();
        total += progress.bytes_read;
        assert_eq!(progress.files_read, 0, "the last file is under way");
        let body = fs::read_to_string(&last).unwrap();
        if rewrite {
            let second = body.lines().nth(1).unwrap().to_owned();
            let changed = second.replacen("synthetic", "SYNTHETIC", 1);
            assert_eq!(changed.len(), second.len());
            fs::write(&last, body.replacen(&second, &changed, 1)).unwrap();
        } else {
            let tail: Value = serde_json::from_str(body.lines().last().unwrap()).unwrap();
            let mut row = message(3_000_000, "appended");
            row["ordinal"] = json!(tail["ordinal"].as_i64().unwrap() + 1);
            let mut file = fs::OpenOptions::new().append(true).open(&last).unwrap();
            std::io::Write::write_all(&mut file, format!("{row}\n").as_bytes()).unwrap();
        }
        let passes = drain(&home, &mut store, &mut backlog, &limits);
        total += passes.iter().map(|pass| pass.bytes_read).sum::<u64>();
        let sum = totals(&passes);
        let history = history_bytes(&files);
        let last_len = fs::metadata(&last).unwrap().len();
        println!("rewrite={rewrite} history={history} total={total} {sum:?}");
        assert_eq!(sum.linked, 1, "rewrite={rewrite}");
        assert_eq!(sum.files_reused, 3, "the unchanged files came from memory");
        assert_eq!(sum.files_read, 1, "only the changed file was read again");
        assert!(
            total < history + last_len + 256 * 1024,
            "{total} vs {history}"
        );
    }
}

/// A parent whose history cannot be read now does not hold up the next:
/// the valid parent queued after it is linked in the same passes, and the
/// unreadable one is kept for a later retry.
#[test]
fn an_unreadable_first_parent_does_not_block_a_valid_one() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    let files = actual_history(&home, exit(0, ""));
    write_child(&home, &child_rows(4203, PROMPT));
    // A second, earlier-sorting thread whose file cannot be opened.
    const BLOCKED: &str = "01000000-0000-7000-8000-000000000001";
    let blocked = home
        .day("2026/09/01")
        .join(format!("rollout-2026-09-01T00-00-00-{BLOCKED}.jsonl"));
    write_rows(
        &blocked,
        &[
            json!({"timestamp": iso(-90_000_000), "type": "session_meta",
            "payload": {"id": BLOCKED, "cwd": "/w", "source": "cli"}}),
        ],
    );
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    let lines = vec![
        json!({"type": "session", "host": "codex", "native_session_id": BLOCKED,
               "conversation_id": format!("codex-{BLOCKED}"), "source_surface": "codex_cli",
               "started_at": iso(-90_000_000), "cwd": "/w", "git_branch": null, "title": null,
               "path": blocked.display().to_string(), "mtime": 1790668716.0})
        .to_string(),
        json!({"uuid": "5555bbbb-5555-4555-8555-000000000000", "type": "user", "cwd": "/w",
               "timestamp": iso(-89_000_000),
               "message": {"role": "user", "content": [{"type": "text", "text": "Other"}]}})
        .to_string(),
    ];
    import_reader_lines(
        &mut store,
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
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
    let mut backlog = LaunchBacklog::starting();
    let mut linked = 0;
    let mut busy = 0;
    for _ in 0..20 {
        let progress = continue_claude_launches(
            &mut store,
            &home.home,
            &mut backlog,
            &LaunchLimits::default(),
            None,
            T,
        )
        .unwrap();
        linked += progress.linked;
        busy += progress.threads_busy;
        if linked > 0 {
            break;
        }
    }
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(linked, 1);
    assert!(busy >= 1, "the unreadable parent was met and rotated");
    assert!(backlog.pending(), "the unreadable parent waits for a retry");
}

/// Several children, each transcript larger than a pass's budget, are
/// checked under the one shared budget, resuming mid-transcript.
#[test]
fn several_children_are_checked_under_a_shared_budget() {
    const CHILDREN: usize = 6;
    let home = Home::new();
    let child = |index: usize| format!("0c000000-0000-4000-8000-{:012x}", 0x200 + index);
    let mut rows = Vec::new();
    for index in 0..CHILDREN {
        let base = index as i64 * 10_000;
        let launch = format!("call_launch_{index}");
        rows.push(call(
            base,
            &launch,
            &launch_cell(&home.claude(), "", &child(index)),
        ));
        rows.push(output(base + 5, &launch, DONE, &[exit(0, "")]));
    }
    let files = write_history(&home, &[vec![message(-3_000_000, "earlier")], rows]);
    let project = home.home.join(".claude/projects/-w");
    fs::create_dir_all(&project).unwrap();
    for index in 0..CHILDREN {
        let native = child(index);
        let at = index as i64 * 10_000 + 2;
        let mut lines = vec![
            json!({"type": "user", "uuid": format!("0d000000-0000-4000-8000-{:012x}", 0x200 + index),
            "parentUuid": null, "timestamp": iso(at), "sessionId": native, "cwd": "/w",
            "message": {"role": "user", "content": PROMPT}}),
        ];
        // About 40 KB of answers after the exit, larger than one pass.
        for n in 0..40 {
            lines.push(json!({"type": "assistant", "uuid": format!("0e000000-0000-4000-8000-{:06x}{:06x}", index, n),
                "timestamp": iso(at + 100 + n), "sessionId": native, "cwd": "/w",
                "message": {"role": "assistant", "id": format!("msg_{index}_{n}"), "model": "fixture-model",
                    "content": [{"type": "text", "text": "answer ".repeat(140)}],
                    "usage": {"input_tokens": 1, "output_tokens": 1}}}));
        }
        write_rows(&project.join(format!("{native}.jsonl")), &lines);
    }
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let budget = 16 * 1024;
    let mut backlog = LaunchBacklog::starting();
    let passes = drain(&home, &mut store, &mut backlog, &tiny(budget));
    let per_pass: Vec<u64> = passes.iter().map(|pass| pass.bytes_read).collect();
    println!(
        "children={CHILDREN} passes={} max_bytes_per_pass={}",
        per_pass.len(),
        per_pass.iter().max().unwrap()
    );
    assert_eq!(
        store.claude_launch_summary().unwrap().relations_accepted,
        CHILDREN as u64
    );
    // A pass reads its budget, plus at most the opening lines of the
    // history or two recorded lines checked as one step.
    assert!(
        per_pass.iter().all(|bytes| *bytes <= budget + 16 * 1024),
        "{per_pass:?}"
    );
    assert!(
        per_pass.len() > CHILDREN * 2,
        "transcripts were read across passes"
    );
}

/// A cancel stops a pass at once: nothing is read, and the work is kept for
/// the next pass, which completes it.
#[test]
fn a_cancel_stops_a_pass_at_once_and_loses_nothing() {
    use xt_ingest::native::readers_cli::CancelToken;
    let home = Home::new();
    let files = large_history(&home, 200);
    write_child(&home, &child_rows(4203, PROMPT));
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let mut backlog = LaunchBacklog::starting();
    // A census pass, then a first read under way.
    for _ in 0..3 {
        continue_claude_launches(
            &mut store,
            &home.home,
            &mut backlog,
            &tiny(32 * 1024),
            None,
            T,
        )
        .unwrap();
    }
    let token = CancelToken::new();
    token.cancel();
    let started = Instant::now();
    let cancelled = continue_claude_launches(
        &mut store,
        &home.home,
        &mut backlog,
        &LaunchLimits::default(),
        Some(&token),
        T,
    )
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(cancelled.bytes_read, 0, "{cancelled:?}");
    assert!(backlog.pending());
    let passes = drain(&home, &mut store, &mut backlog, &LaunchLimits::default());
    assert_eq!(passes.iter().map(|pass| pass.linked).sum::<usize>(), 1);
}

/// A child transcript larger than a tiny pass: the launch's first input,
/// then ~60 KB of answers after the exit.
fn large_child_rows() -> Vec<Value> {
    let mut rows = child_rows(4203, PROMPT);
    for n in 0..60 {
        rows.push(
            json!({"type": "assistant", "uuid": format!("0e000000-0000-4000-8000-{n:012x}"),
            "timestamp": iso(946_000 + n), "sessionId": CHILD, "cwd": "/w",
            "message": {"role": "assistant", "id": format!("msg_{n}"), "model": "fixture-model",
                "content": [{"type": "text", "text": "answer ".repeat(140)}],
                "usage": {"input_tokens": 1, "output_tokens": 1}}}),
        );
    }
    rows
}

/// Append one more continuation to a written history, continuing the whole
/// last file. Returns its path.
fn add_continuation(home: &Home, files: &mut Vec<PathBuf>, body: Vec<Value>) -> PathBuf {
    let index = files.len();
    let last = files.last().unwrap();
    let text = fs::read_to_string(last).unwrap();
    let tail: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    let mut ordinal = tail["ordinal"].as_i64().unwrap() + 1;
    let base_rollout = if index == 1 {
        PARENT
    } else {
        ROLLOUTS[index - 2]
    };
    let rollout = ROLLOUTS[index - 1];
    let path = home
        .day(&format!("2026/09/{}", 10 + 6 * index))
        .join(format!(
            "rollout-2026-09-{}T16-14-04-{PARENT}_{rollout}.jsonl",
            10 + 6 * index
        ));
    let mut rows = vec![
        json!({"timestamp": iso(-86_400_000 + index as i64), "type": "session_meta",
        "ordinal": ordinal, "payload": {"id": PARENT, "originator": "codex_cli_rs",
        "cwd": "/w", "source": "cli", "history_mode": "paginated",
        "history_base": {"thread_id": base_rollout, "end_byte_offset": text.len(),
            "end_ordinal_exclusive": ordinal}}}),
    ];
    ordinal += 1;
    for row in body {
        let mut row = row;
        row["ordinal"] = json!(ordinal);
        ordinal += 1;
        rows.push(row);
    }
    write_rows(&path, &rows);
    files.push(path.clone());
    path
}

/// Two rows that reuse `id`: a call and its output.
fn reuse(id: &str, offset: i64) -> Vec<Value> {
    vec![
        call(offset, id, &check_cell()),
        output(offset + 1, id, DONE, &[exit(0, "")]),
    ]
}

/// A prepared parent (actual shape) and child, with the launch found and
/// its child check paused mid-transcript under a tiny budget.
fn paused_child_check(home: &Home) -> (Store, LaunchBacklog, Vec<PathBuf>, LaunchLimits) {
    let files = actual_history(home, exit(0, ""));
    write_child(home, &large_child_rows());
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(home, &mut store, &mut SpawnBacklog::default());
    let limits = tiny(8 * 1024);
    let mut backlog = LaunchBacklog::starting();
    for _ in 0..10_000 {
        continue_claude_launches(&mut store, &home.home, &mut backlog, &limits, None, T).unwrap();
        if store.claude_launch_summary().unwrap().groups_valid == 1 {
            break;
        }
    }
    // Two more passes: the child's transcript is being read.
    for _ in 0..2 {
        let pass = continue_claude_launches(&mut store, &home.home, &mut backlog, &limits, None, T)
            .unwrap();
        assert_eq!(pass.linked, 0);
        assert!(pass.threads_paused == 1 && pass.bytes_read > 0, "{pass:?}");
    }
    assert_eq!(store.claude_launch_summary().unwrap().relations_accepted, 0);
    (store, backlog, files, limits)
}

/// A child check paused mid-transcript, and meanwhile: nothing (control),
/// a same-length rewrite of the launch's own reply to name another process,
/// a change to another member, a new continuation, or one reusing the
/// launch's call identifier. Only the control links from the first
/// validation; a benign change, or the rewrite, links only from a new one,
/// as the history now is; the reuse never.
#[test]
fn a_paused_child_check_links_only_against_the_current_whole_history() {
    for case in [
        "control",
        "reply_rewrite",
        "other_member",
        "new_continuation",
        "duplicate_continuation",
    ] {
        let home = Home::new();
        let (mut store, mut backlog, mut files, limits) = paused_child_check(&home);
        let first_revision = store.claude_launch_group(PARENT).unwrap().unwrap().revision;
        match case {
            "reply_rewrite" => {
                let last = files.last().unwrap();
                let body = fs::read_to_string(last).unwrap();
                let rewritten =
                    body.replacen("\\\"session_id\\\":56751", "\\\"session_id\\\":56752", 1);
                assert_ne!(rewritten, body);
                assert_eq!(rewritten.len(), body.len());
                fs::write(last, rewritten).unwrap();
            }
            "other_member" => {
                let body = fs::read_to_string(&files[1]).unwrap();
                let second = body.lines().nth(1).unwrap().to_owned();
                let changed = second.replacen("Earlier", "EARLIER", 1);
                fs::write(&files[1], body.replacen(&second, &changed, 1)).unwrap();
            }
            "new_continuation" => {
                add_continuation(&home, &mut files, vec![message(3_000_000, "later work")]);
                backlog.add_threads([PARENT]);
            }
            "duplicate_continuation" => {
                // The launch's call identifier, used again.
                add_continuation(&home, &mut files, reuse("call_launch", 3_000_000));
                backlog.add_threads([PARENT]);
            }
            _ => {}
        }
        let passes = drain(&home, &mut store, &mut backlog, &limits);
        let sum = totals(&passes);
        let child = child_session(&store).unwrap();
        let linked = store.claude_launch_creation(&child).unwrap();
        let revision = store.claude_launch_group(PARENT).unwrap().unwrap().revision;
        match case {
            "control" => {
                assert!(linked.is_some(), "{case}: {sum:?}");
                assert_eq!(revision, first_revision, "{case}: no second validation");
            }
            "other_member" | "new_continuation" | "reply_rewrite" => {
                let (proof, _) = linked.unwrap_or_else(|| panic!("{case}: {sum:?}"));
                assert!(
                    revision > first_revision,
                    "{case}: linked from a new validation"
                );
                let handle = if case == "reply_rewrite" {
                    "56752"
                } else {
                    "56751"
                };
                assert_eq!(proof.process_session_id.as_deref(), Some(handle));
            }
            _ => assert!(linked.is_none(), "{case}: {sum:?}"),
        }
        assert!(
            passes
                .iter()
                .all(|pass| pass.bytes_read <= limits.max_pass_bytes)
        );
    }
}

/// The launch's own call identifier used again in another member, before or
/// after the launch's member: no link, also after a restart and after the
/// child is imported again. The later calls of the job — its completion, a
/// poll, a wait — are not part of the acknowledgment: their identifiers
/// reused still link, as an unrelated one does.
#[test]
fn a_launch_identifier_used_again_anywhere_in_the_history_links_nothing() {
    for id in [
        "call_launch",
        "call_completion",
        "call_poll_1",
        "call_wait_2",
        "call_unrelated",
    ] {
        for earlier in [true, false] {
            let home = Home::new();
            let launch = launch_rows(&home.claude(), CHILD, exit(0, ""));
            let other = {
                let mut rows = vec![message(-3_000_000, "earlier")];
                rows.extend(reuse(id, if earlier { -2_900_000 } else { 3_000_000 }));
                rows
            };
            let bodies = if earlier {
                vec![vec![message(-3_100_000, "first")], other, launch]
            } else {
                vec![vec![message(-3_100_000, "first")], launch, other]
            };
            let files = write_history(&home, &bodies);
            write_child(&home, &child_rows(4203, PROMPT));
            let mut store = home.store();
            index_parent(&mut store, &files[0]);
            scan_claude(&home, &mut store, &mut SpawnBacklog::default());
            let control = id != "call_launch";
            let check = |store: &Store, when: &str| {
                let child = child_session(store).unwrap();
                assert_eq!(
                    store.claude_launch_creation(&child).unwrap().is_some(),
                    control,
                    "{id} earlier={earlier} {when}"
                );
            };
            let mut backlog = LaunchBacklog::starting();
            drain(&home, &mut store, &mut backlog, &LaunchLimits::default());
            check(&store, "first");
            // A restart.
            drop(store);
            let mut store = home.store();
            let mut backlog = LaunchBacklog::starting();
            drain(&home, &mut store, &mut backlog, &LaunchLimits::default());
            check(&store, "restart");
            // The child imported again.
            let mut spawns = SpawnBacklog::default();
            fs::OpenOptions::new()
                .append(true)
                .open(home.home.join(format!(".claude/projects/-w/{CHILD}.jsonl")))
                .and_then(|mut file| {
                    std::io::Write::write_all(
                        &mut file,
                        format!("{}\n", json!({"type": "assistant", "uuid": "0e000000-0000-4000-8000-00000000abcd",
                            "timestamp": iso(3_000_000), "sessionId": CHILD, "cwd": "/w",
                            "message": {"role": "assistant", "id": "msg_more", "model": "fixture-model",
                                "content": [{"type": "text", "text": "more"}],
                                "usage": {"input_tokens": 1, "output_tokens": 1}}})).as_bytes(),
                    )
                })
                .unwrap();
            scan_claude(&home, &mut store, &mut spawns);
            let mut backlog = LaunchBacklog::starting();
            backlog.add_children([CHILD]);
            drain(&home, &mut store, &mut backlog, &LaunchLimits::default());
            check(&store, "re-import");
        }
    }
}

/// A member with a malformed row makes the history invalid: no launch, and
/// not read again while unchanged. A valid member without launches is an
/// ordinary valid member.
#[test]
fn an_invalid_member_is_not_a_valid_empty_member() {
    for invalid in [true, false] {
        let home = Home::new();
        let mut files = write_history(
            &home,
            &[
                vec![message(-3_000_000, "earlier")],
                vec![message(-2_000_000, "nothing"), message(-1_999_000, "tail")],
            ],
        );
        if invalid {
            // A malformed row in the middle of the member.
            let body = fs::read_to_string(&files[1]).unwrap();
            let row = body.lines().nth(1).unwrap().to_owned();
            fs::write(&files[1], body.replacen(&row, "{not json", 1)).unwrap();
        }
        add_continuation(
            &home,
            &mut files,
            launch_rows(&home.claude(), CHILD, exit(0, "")),
        );
        write_child(&home, &child_rows(4203, PROMPT));
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
        scan_claude(&home, &mut store, &mut SpawnBacklog::default());
        let mut backlog = LaunchBacklog::starting();
        let first = totals(&drain(
            &home,
            &mut store,
            &mut backlog,
            &LaunchLimits::default(),
        ));
        let summary = store.claude_launch_summary().unwrap();
        if invalid {
            assert_eq!(
                (summary.groups_invalid, summary.members_invalid),
                (1, 1),
                "{summary:?}"
            );
            assert_eq!((first.linked, summary.candidates_waiting), (0, 0));
        } else {
            assert_eq!(
                (summary.groups_valid, summary.members_invalid),
                (1, 0),
                "{summary:?}"
            );
            assert_eq!(first.linked, 1);
        }
        // Unchanged: not read again.
        let mut backlog = LaunchBacklog::starting();
        let again = totals(&drain(
            &home,
            &mut store,
            &mut backlog,
            &LaunchLimits::default(),
        ));
        assert_eq!((again.bytes_read, again.files_read), (0, 0), "{again:?}");
    }
}

/// More than one page of launches: 1,100 launches in one member, one of
/// them duplicated in another member. Every other launch is linked; the
/// duplicate is not.
#[test]
fn more_than_a_thousand_launches_are_all_looked_at() {
    const COUNT: usize = 1_100;
    let home = Home::new();
    let child = |index: usize| format!("0c000000-0000-4000-8000-{:012x}", 0x1000 + index);
    let mut rows = Vec::new();
    for index in 0..COUNT {
        let base = index as i64 * 100;
        let launch = format!("call_launch_{index:04}");
        rows.push(call(
            base,
            &launch,
            &launch_cell(&home.claude(), "", &child(index)),
        ));
        rows.push(output(base + 5, &launch, DONE, &[exit(0, "")]));
    }
    let files = write_history(&home, &[reuse("call_launch_1050", -3_000_000), rows]);
    let project = home.home.join(".claude/projects/-w");
    fs::create_dir_all(&project).unwrap();
    for index in 0..COUNT {
        let native = child(index);
        let row = json!({"type": "user", "uuid": format!("0d000000-0000-4000-8000-{:012x}", 0x1000 + index),
            "parentUuid": null, "timestamp": iso(index as i64 * 100 + 2), "sessionId": native, "cwd": "/w",
            "message": {"role": "user", "content": PROMPT}});
        write_rows(&project.join(format!("{native}.jsonl")), &[row]);
    }
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let mut backlog = LaunchBacklog::starting();
    let passes = drain(&home, &mut store, &mut backlog, &LaunchLimits::default());
    let summary = store.claude_launch_summary().unwrap();
    assert_eq!(
        summary.relations_accepted,
        (COUNT - 1) as u64,
        "{summary:?}"
    );
    let duplicate = store
        .user_sessions_with_native(Host::Claude, &child(1050))
        .unwrap();
    assert!(store.session_creation(&duplicate[0]).unwrap().is_none());
    assert!(passes.len() > 1, "resolution rotated across passes");
}

/// An unreadable directory in the history root: no history is used from
/// that census, and once it is readable again a later pass, with no new
/// event, takes the census again and links.
#[test]
fn a_census_that_could_not_read_a_directory_is_taken_again() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    let files = actual_history(&home, exit(0, ""));
    write_child(&home, &child_rows(4203, PROMPT));
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let blocked = files[2].parent().unwrap().to_path_buf();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
    let mut backlog = LaunchBacklog::starting();
    let mut waiting = Vec::new();
    for _ in 0..5 {
        waiting.push(
            continue_claude_launches(
                &mut store,
                &home.home,
                &mut backlog,
                &LaunchLimits::default(),
                None,
                T,
            )
            .unwrap(),
        );
    }
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        waiting
            .iter()
            .all(|pass| pass.linked == 0 && pass.census_pending),
        "{waiting:?}"
    );
    assert!(backlog.pending(), "the census is taken again later");
    assert_eq!(store.claude_launch_summary().unwrap().members, 0);
    let passes = drain(&home, &mut store, &mut backlog, &LaunchLimits::default());
    assert_eq!(totals(&passes).linked, 1);
}

/// A new continuation appears while a member read is paused: the attempt
/// cannot publish without it. With the launch's call identifier reused in
/// it, nothing is linked; a benign one is included and the launch linked.
#[test]
fn a_new_continuation_during_a_paused_read_is_part_of_the_validation() {
    for duplicate in [false, true] {
        let home = Home::new();
        let mut files = large_history(&home, 300);
        write_child(&home, &child_rows(4203, PROMPT));
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
        scan_claude(&home, &mut store, &mut SpawnBacklog::default());
        let limits = tiny(64 * 1024);
        let mut backlog = LaunchBacklog::starting();
        let mut read = 0;
        for _ in 0..10_000 {
            let pass =
                continue_claude_launches(&mut store, &home.home, &mut backlog, &limits, None, T)
                    .unwrap();
            read += pass.files_read;
            if read >= 1 {
                break;
            }
        }
        assert_eq!(store.claude_launch_summary().unwrap().groups_pending, 1);
        let body = if duplicate {
            reuse("call_launch", 3_000_000)
        } else {
            vec![message(3_000_000, "later")]
        };
        add_continuation(&home, &mut files, body);
        backlog.add_threads([PARENT]);
        let sum = totals(&drain(&home, &mut store, &mut backlog, &limits));
        let summary = store.claude_launch_summary().unwrap();
        assert_eq!(summary.members, 5, "the new continuation is a member");
        assert_eq!(
            sum.linked,
            usize::from(!duplicate),
            "duplicate={duplicate} {sum:?}"
        );
    }
}

/// Several launches whose cells are still running at once, each cell's
/// first result large: a launch waiting for its result keeps only a count of
/// the results before it, never the results. Under a small per-thread
/// allowance they all link, at once or one after another; under an
/// allowance too small for the history's own rows it is refused (recorded
/// invalid, never a partial link).
#[test]
fn simultaneous_launches_keep_no_results_and_an_allowance_too_small_refuses() {
    const CHAINS: usize = 5;
    let child = |index: usize| format!("0c000000-0000-4000-8000-{:012x}", 0x300 + index);
    let big = "x".repeat(40_000);
    let two_ops = |claude: &str, index: usize| {
        format!(
            "{};\n{}",
            check_cell(),
            launch_cell(claude, "", &child(index)).replacen(
                "const p",
                "const q = \"\";\nconst p",
                1
            )
        )
    };
    for (simultaneous, memory, expect_linked) in [
        (true, 300 * 1024, true),
        (false, 300 * 1024, true),
        (true, 64 * 1024, false),
    ] {
        let home = Home::new();
        let mut rows = Vec::new();
        // Each launch's cell has its own identifier.
        let running = |id: &str, cell: usize, offset: i64, results: &[Value]| {
            let mut items = vec![
                json!({"type": "input_text", "text": format!("Script running with cell ID {cell}\n")}),
            ];
            items.extend(
                results
                    .iter()
                    .map(|r| json!({"type": "input_text", "text": r.to_string()})),
            );
            json!({"timestamp": iso(offset), "type": "response_item", "payload": {
                "type": "custom_tool_call_output", "call_id": id, "output": items}})
        };
        let opens: Vec<Vec<Value>> = (0..CHAINS)
            .map(|index| {
                let id = format!("call_launch_{index}");
                vec![
                    call(index as i64 * 10, &id, &two_ops(&home.claude(), index)),
                    running(&id, 9 + index, index as i64 * 10 + 1, &[exit(0, &big)]),
                ]
            })
            .collect();
        let closes: Vec<Vec<Value>> = (0..CHAINS)
            .map(|index| {
                let id = format!("call_launch_{index}");
                let wait_id = format!("call_wait_{index}");
                vec![
                    wait(1000 + index as i64 * 10, &wait_id, &(9 + index).to_string()),
                    output(1001 + index as i64 * 10, &wait_id, DONE, &[exit(0, "")]),
                ]
                .into_iter()
                .chain(std::iter::once(message(1002 + index as i64 * 10, &id)))
                .collect()
            })
            .collect();
        if simultaneous {
            rows.extend(opens.iter().flatten().cloned());
            rows.extend(closes.iter().flatten().cloned());
        } else {
            for (open, close) in opens.iter().zip(&closes) {
                rows.extend(open.iter().cloned());
                rows.extend(close.iter().cloned());
            }
        }
        let files = write_history(&home, &[vec![message(-3_000_000, "earlier")], rows]);
        let project = home.home.join(".claude/projects/-w");
        fs::create_dir_all(&project).unwrap();
        for index in 0..CHAINS {
            let native = child(index);
            let row = json!({"type": "user", "uuid": format!("0d000000-0000-4000-8000-{:012x}", 0x300 + index),
                "parentUuid": null, "timestamp": iso(index as i64 * 10 + 2), "sessionId": native, "cwd": "/w",
                "message": {"role": "user", "content": PROMPT}});
            write_rows(&project.join(format!("{native}.jsonl")), &[row]);
        }
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
        scan_claude(&home, &mut store, &mut SpawnBacklog::default());
        let limits = LaunchLimits {
            thread_memory: memory,
            ..LaunchLimits::default()
        };
        let mut backlog = LaunchBacklog::starting();
        let sum = totals(&drain(&home, &mut store, &mut backlog, &limits));
        let summary = store.claude_launch_summary().unwrap();
        if expect_linked {
            assert_eq!(
                summary.relations_accepted, CHAINS as u64,
                "{simultaneous} {memory} {sum:?}"
            );
        } else {
            assert_eq!(
                (summary.relations_accepted, summary.groups_invalid),
                (0, 1),
                "{summary:?}"
            );
        }
    }
}

/// Three parents queued at once with room for two: the third waits its
/// turn, then all three are linked.
#[test]
fn a_third_thread_waits_for_a_slot_and_is_linked() {
    let home = Home::new();
    let parents = [
        "01a00000-0000-7000-8000-00000000a001",
        "01a00000-0000-7000-8000-00000000a002",
        "01a00000-0000-7000-8000-00000000a003",
    ];
    let children = [
        "0c000000-0000-4000-8000-00000000c001",
        "0c000000-0000-4000-8000-00000000c002",
        "0c000000-0000-4000-8000-00000000c003",
    ];
    let mut store = home.store();
    let project = home.home.join(".claude/projects/-w");
    fs::create_dir_all(&project).unwrap();
    for (index, (parent, child)) in parents.iter().zip(children).enumerate() {
        let path = home
            .day("2026/09/07")
            .join(format!("rollout-2026-09-07T0{index}-00-00-{parent}.jsonl"));
        let mut rows = vec![
            json!({"timestamp": iso(-86_400_000), "type": "session_meta",
            "payload": {"id": parent, "cwd": "/w", "source": "cli"}}),
        ];
        for n in 0..300 {
            rows.push(message(-3_000_000 + n, &"filler ".repeat(100)));
        }
        rows.push(call(
            0,
            "call_launch",
            &launch_cell(&home.claude(), "", child),
        ));
        rows.push(output(5, "call_launch", DONE, &[exit(0, "")]));
        write_rows(&path, &rows);
        let lines = vec![
            json!({"type": "session", "host": "codex", "native_session_id": parent,
                   "conversation_id": format!("codex-{parent}"), "source_surface": "codex_cli",
                   "started_at": iso(-86_400_000), "cwd": "/w", "git_branch": null, "title": null,
                   "path": path.display().to_string(), "mtime": 1790668716.0})
            .to_string(),
            json!({"uuid": format!("5555aaaa-5555-4555-8555-00000000000{index}"), "type": "user", "cwd": "/w",
                   "timestamp": iso(-2500),
                   "message": {"role": "user", "content": [{"type": "text", "text": "Synthetic request"}]}})
            .to_string(),
        ];
        import_reader_lines(
            &mut store,
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
        write_rows(
            &project.join(format!("{child}.jsonl")),
            &[
                json!({"type": "user", "uuid": format!("0d000000-0000-4000-8000-00000000d00{index}"),
                "parentUuid": null, "timestamp": iso(2), "sessionId": child, "cwd": "/w",
                "message": {"role": "user", "content": PROMPT}}),
            ],
        );
    }
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let mut backlog = LaunchBacklog::starting();
    let passes = drain(&home, &mut store, &mut backlog, &tiny(16 * 1024));
    let sum = totals(&passes);
    let waited: usize = passes.iter().map(|pass| pass.threads_waiting).sum();
    assert!(waited > 0, "the third thread waited: {sum:?}");
    assert_eq!(store.claude_launch_summary().unwrap().relations_accepted, 3);
}

/// Run `count` passes of the production drainer with `limits`, whatever is
/// pending. Returns every pass.
fn passes(
    home: &Home,
    store: &mut Store,
    backlog: &mut LaunchBacklog,
    limits: &LaunchLimits,
    count: usize,
) -> Vec<LaunchProgress> {
    (0..count)
        .map(|_| continue_claude_launches(store, &home.home, backlog, limits, None, T).unwrap())
        .collect()
}

/// Append `row` to `path` with its ordinal after the file's last one, and
/// without a final newline: a row still being written.
fn append_unfinished(path: &Path, mut row: Value) {
    let body = fs::read_to_string(path).unwrap();
    let last: Value = serde_json::from_str(body.lines().last().unwrap()).unwrap();
    if let Some(ordinal) = last["ordinal"].as_i64() {
        row["ordinal"] = json!(ordinal + 1);
    }
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    std::io::Write::write_all(&mut file, row.to_string().as_bytes()).unwrap();
}

fn finish_row(path: &Path) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    std::io::Write::write_all(&mut file, b"\n").unwrap();
}

/// A parent history whose last row has no final newline yet: nothing is
/// published or linked from the complete prefix, the unchanged unfinished
/// file is not read again pass after pass, and once the newline arrives —
/// with no other event — the whole file decides. A duplicate of the launch's
/// call identifier in that row links nothing; a benign row links.
#[test]
fn an_unfinished_last_parent_row_holds_the_history_until_it_is_complete() {
    for duplicate in [false, true] {
        let home = Home::new();
        let files = actual_history(&home, exit(0, ""));
        let last = files.last().unwrap();
        let row = if duplicate {
            call(3_000_000, "call_launch", &check_cell())
        } else {
            message(3_000_000, "still writing")
        };
        append_unfinished(last, row);
        write_child(&home, &child_rows(4203, PROMPT));
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
        scan_claude(&home, &mut store, &mut SpawnBacklog::default());
        let mut backlog = LaunchBacklog::starting();
        let held = passes(
            &home,
            &mut store,
            &mut backlog,
            &LaunchLimits::default(),
            30,
        );
        let child = child_session(&store).unwrap();
        assert!(
            store.claude_launch_creation(&child).unwrap().is_none(),
            "duplicate={duplicate}: linked from an unfinished history {:?}",
            totals(&held)
        );
        let summary = store.claude_launch_summary().unwrap();
        assert_eq!(
            (summary.groups_valid, summary.groups_pending),
            (0, 1),
            "duplicate={duplicate}: {summary:?}"
        );
        assert!(backlog.pending(), "the unfinished history is retried");
        assert!(
            held[10..].iter().all(|pass| pass.bytes_read == 0),
            "duplicate={duplicate}: no spin on an unchanged unfinished tail {:?}",
            held.iter().map(|pass| pass.bytes_read).collect::<Vec<_>>()
        );
        finish_row(last);
        let done = totals(&passes(
            &home,
            &mut store,
            &mut backlog,
            &LaunchLimits::default(),
            30,
        ));
        assert_eq!(
            store.claude_launch_creation(&child).unwrap().is_some(),
            !duplicate,
            "duplicate={duplicate}: {done:?}"
        );
        assert_eq!(store.claude_launch_summary().unwrap().groups_valid, 1);
        if duplicate {
            assert!(done.launches_broken >= 1, "{done:?}");
        }
    }
}

/// A child transcript whose last row has no final newline yet: no link from
/// the prefix, the unchanged transcript is not read again pass after pass,
/// and once the newline arrives the whole transcript decides. A second input
/// at the first input's own time (so the first is not unique) rejects; a
/// benign row links.
#[test]
fn an_unfinished_last_child_row_holds_the_check_until_it_is_complete() {
    for contradiction in [false, true] {
        let home = Home::new();
        let files = actual_history(&home, exit(0, ""));
        let transcript = write_child(&home, &child_rows(4203, PROMPT));
        let row = if contradiction {
            json!({"type": "user", "uuid": "0d000000-0000-4000-8000-0000000000e9",
                "parentUuid": null, "timestamp": iso(4203), "sessionId": CHILD, "cwd": "/w",
                "message": {"role": "user", "content": "Another request"}})
        } else {
            json!({"type": "assistant", "uuid": "0d000000-0000-4000-8000-0000000000e9",
                "parentUuid": FIRST, "timestamp": iso(2_700_000), "sessionId": CHILD, "cwd": "/w",
                "message": {"role": "assistant", "id": "msg_tail", "model": "fixture-model",
                    "content": [{"type": "text", "text": "tail"}],
                    "usage": {"input_tokens": 1, "output_tokens": 1}}})
        };
        append_unfinished(&transcript, row);
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
        scan_claude(&home, &mut store, &mut SpawnBacklog::default());
        let child = child_session(&store).unwrap();
        let mut backlog = LaunchBacklog::starting();
        let held = passes(
            &home,
            &mut store,
            &mut backlog,
            &LaunchLimits::default(),
            30,
        );
        assert!(
            store.claude_launch_creation(&child).unwrap().is_none(),
            "contradiction={contradiction}: linked from an unfinished transcript {:?}",
            totals(&held)
        );
        assert!(backlog.pending(), "the unfinished child is retried");
        assert!(
            held[10..].iter().all(|pass| pass.bytes_read == 0),
            "contradiction={contradiction}: no spin {:?}",
            held.iter().map(|pass| pass.bytes_read).collect::<Vec<_>>()
        );
        finish_row(&transcript);
        let done = totals(&passes(
            &home,
            &mut store,
            &mut backlog,
            &LaunchLimits::default(),
            30,
        ));
        assert_eq!(
            store.claude_launch_creation(&child).unwrap().is_some(),
            !contradiction,
            "contradiction={contradiction}: {done:?}"
        );
        let rows = store
            .claude_launch_candidates_for_child(CHILD, None, 10)
            .unwrap();
        let expected = if contradiction {
            ChildState::Rejected
        } else {
            ChildState::Linked
        };
        assert_eq!(rows[0].child_state, expected, "{rows:?}");
    }
}

/// Children launched by the rows of one parent: each child's transcript is
/// one first input equal to the prompt at offset `first`. The parent and
/// children are indexed. Returns the store.
fn launch_home(home: &Home, rows: Vec<Value>, children: &[String], first: i64) -> Store {
    let files = write_history(home, &[vec![message(-3_000_000, "earlier")], rows]);
    let project = home.home.join(".claude/projects/-w");
    fs::create_dir_all(&project).unwrap();
    for (index, native) in children.iter().enumerate() {
        let row = json!({"type": "user", "uuid": format!("0d000000-0000-4000-8000-{:012x}", 0x500 + index),
            "parentUuid": null, "timestamp": iso(first), "sessionId": native, "cwd": "/w",
            "message": {"role": "user", "content": PROMPT}});
        write_rows(&project.join(format!("{native}.jsonl")), &[row]);
    }
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(home, &mut store, &mut SpawnBacklog::default());
    store
}

/// One cell of several operations, each launching its own child as
/// `launch_cell` does.
fn launches_cell(claude: &str, children: &[String]) -> String {
    let mut cell = format!("const p = {};\n", js(PROMPT));
    for (index, child) in children.iter().enumerate() {
        let prefix = format!("{claude} -p --session-id {child} --output-format json '");
        cell.push_str(&format!(
            "const r{index} = await tools.exec_command({{cmd: {} + p.replace(/'/g, \"'\\\\''\") + {}, workdir: \"/w\"}});\ntext(r{index});\n",
            js(&prefix),
            js(&format!("' > /w/result{index}.json"))
        ));
    }
    cell
}

/// A check, then a launch of `child`: the launch is operation 1.
fn check_then_launch(claude: &str, child: &str) -> String {
    format!(
        "{};\n{}",
        check_cell(),
        launch_cell(claude, "", child).replacen("const p", "const q = \"\";\nconst p", 1)
    )
}

fn running(offset: i64, id: &str, cell: &str, results: &[Value]) -> Value {
    output(
        offset,
        id,
        &format!("Script running with cell ID {cell}\n"),
        results,
    )
}

/// Running cells belong to the exec call that announced them. Operations of
/// one exec call share its cell and waits and keep their positions: both
/// link from their own results, with no poll. Two exec calls claiming one
/// running cell: nothing links. A handle number is not an identity: two
/// operations or two launch calls acknowledged with the same number each
/// link their own child.
#[test]
fn running_cells_are_owned_by_one_call_and_handles_are_not_identities() {
    let children: Vec<String> = (0..2)
        .map(|index| format!("0c000000-0000-4000-8000-{:012x}", 0x400 + index))
        .collect();
    let cases: Vec<(&str, u64)> = vec![
        ("same_call_distinct_processes", 2),
        ("distinct_cells", 2),
        ("one_cell_two_calls", 0),
        ("one_process_two_operations", 2),
        ("one_process_two_calls", 2),
    ];
    for (case, expected) in cases {
        let home = Home::new();
        let claude = home.claude();
        let rows = match case {
            "same_call_distinct_processes" => vec![
                call(0, "call_pair", &launches_cell(&claude, &children)),
                running(200, "call_pair", "21", &[live(4101)]),
                wait(300, "call_pair_wait", "21"),
                output(400, "call_pair_wait", DONE, &[live(4102)]),
            ],
            "distinct_cells" | "one_cell_two_calls" => {
                let second = if case == "distinct_cells" { "10" } else { "9" };
                let mut rows = vec![
                    call(0, "call_one", &check_then_launch(&claude, &children[0])),
                    running(1, "call_one", "9", &[exit(0, "")]),
                    call(10, "call_two", &check_then_launch(&claude, &children[1])),
                    running(11, "call_two", second, &[exit(0, "")]),
                    wait(1000, "call_wait_one", "9"),
                    output(1001, "call_wait_one", DONE, &[exit(0, "")]),
                ];
                if case == "distinct_cells" {
                    rows.push(wait(1010, "call_wait_two", "10"));
                    rows.push(output(1011, "call_wait_two", DONE, &[exit(0, "")]));
                }
                rows
            }
            "one_process_two_operations" => vec![
                call(0, "call_pair", &launches_cell(&claude, &children)),
                output(200, "call_pair", DONE, &[live(HANDLE), live(HANDLE)]),
                call(60_000, "call_poll", &poll_cell(HANDLE)),
                output(60_100, "call_poll", DONE, &[exit(0, "")]),
            ],
            "one_process_two_calls" => vec![
                call(0, "call_a", &launch_cell(&claude, "", &children[0])),
                output(200, "call_a", DONE, &[live(HANDLE)]),
                call(300, "call_b", &launch_cell(&claude, "", &children[1])),
                output(400, "call_b", DONE, &[live(HANDLE)]),
                call(60_000, "call_poll", &poll_cell(HANDLE)),
                output(60_100, "call_poll", DONE, &[exit(0, "")]),
            ],
            _ => unreachable!(),
        };
        let mut store = launch_home(&home, rows, &children, 500);
        let mut backlog = LaunchBacklog::starting();
        let sum = totals(&drain(
            &home,
            &mut store,
            &mut backlog,
            &LaunchLimits::default(),
        ));
        assert_eq!(
            store.claude_launch_summary().unwrap().relations_accepted,
            expected,
            "{case}: {sum:?}"
        );
    }
}

/// A launch whose child is missing is passed over, and another launch's
/// large child check pauses; the missing child is imported meanwhile. Its
/// wakeup is kept: once the paused check finishes, the passed-over launch is
/// looked at again and linked, with no further event or restart.
#[test]
fn a_child_imported_during_another_paused_check_is_linked() {
    let home = Home::new();
    let late = "0c000000-0000-4000-8000-0000000000a1";
    let rows = vec![
        call(0, "call_launch_a", &launch_cell(&home.claude(), "", late)),
        output(5, "call_launch_a", DONE, &[exit(0, "")]),
        call(
            100,
            "call_launch_b",
            &launch_cell(&home.claude(), "", CHILD),
        ),
        output(105, "call_launch_b", DONE, &[exit(0, "")]),
    ];
    let files = write_history(&home, &[vec![message(-3_000_000, "earlier")], rows]);
    // Only the second launch's child exists, larger than several passes.
    let mut large = vec![json!({"type": "user", "uuid": FIRST, "parentUuid": null,
        "timestamp": iso(102), "sessionId": CHILD, "cwd": "/w",
        "message": {"role": "user", "content": PROMPT}})];
    large.extend(large_child_rows().into_iter().skip(4));
    write_child(&home, &large);
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let limits = tiny(8 * 1024);
    let mut backlog = LaunchBacklog::starting();
    let mut waited = false;
    for _ in 0..10_000 {
        let pass = continue_claude_launches(&mut store, &home.home, &mut backlog, &limits, None, T)
            .unwrap();
        waited |= pass.waiting > 0;
        if store.claude_launch_summary().unwrap().groups_valid == 1 {
            break;
        }
    }
    // Two more passes: the second launch's child transcript is being read.
    for _ in 0..2 {
        let pass = continue_claude_launches(&mut store, &home.home, &mut backlog, &limits, None, T)
            .unwrap();
        waited |= pass.waiting > 0;
        assert_eq!(pass.linked, 0, "{pass:?}");
        assert!(pass.threads_paused == 1 && pass.bytes_read > 0, "{pass:?}");
    }
    assert!(waited, "the first launch's child was missing");
    let rows = store
        .claude_launch_candidates_for_child(late, None, 10)
        .unwrap();
    assert_eq!(rows[0].child_state, ChildState::Waiting);
    // The missing child arrives while the other check is paused.
    let project = home.home.join(".claude/projects/-w");
    write_rows(
        &project.join(format!("{late}.jsonl")),
        &[
            json!({"type": "user", "uuid": "0d000000-0000-4000-8000-0000000000a2",
            "parentUuid": null, "timestamp": iso(2), "sessionId": late, "cwd": "/w",
            "message": {"role": "user", "content": PROMPT}}),
        ],
    );
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    backlog.add_children([late]);
    let sum = totals(&drain(&home, &mut store, &mut backlog, &limits));
    assert_eq!(
        store.claude_launch_summary().unwrap().relations_accepted,
        2,
        "{sum:?}"
    );
    assert!(!backlog.pending());
}

/// The child's uniqueness walk over the Claude projects that cannot list
/// them now is retried, not a rejection: once the projects can be listed
/// again the launch links, with no further event.
#[test]
fn a_uniqueness_walk_that_cannot_list_the_projects_is_retried() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    let files = actual_history(&home, exit(0, ""));
    write_child(&home, &child_rows(4203, PROMPT));
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let projects = home.home.join(".claude/projects");
    // Searchable (the transcript opens) but not listable.
    fs::set_permissions(&projects, fs::Permissions::from_mode(0o300)).unwrap();
    let mut backlog = LaunchBacklog::starting();
    let blocked = totals(&passes(
        &home,
        &mut store,
        &mut backlog,
        &LaunchLimits::default(),
        20,
    ));
    fs::set_permissions(&projects, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(blocked.linked, 0, "{blocked:?}");
    let rows = store
        .claude_launch_candidates_for_child(CHILD, None, 10)
        .unwrap();
    assert_eq!(rows[0].child_state, ChildState::Retry, "{rows:?}");
    let sum = totals(&passes(
        &home,
        &mut store,
        &mut backlog,
        &LaunchLimits::default(),
        20,
    ));
    let child = child_session(&store).unwrap();
    assert!(
        store.claude_launch_creation(&child).unwrap().is_some(),
        "{sum:?}"
    );
}

/// One large history directory and many Claude projects, under a pass that
/// may look at 50 directory entries: every pass looks at no more, the census
/// and the check that no other project holds the child's file each continue
/// where they stopped, and the launch links. A cancelled pass looks at none.
#[test]
fn directory_walks_keep_their_place_under_a_tiny_entry_allowance() {
    use xt_ingest::native::readers_cli::CancelToken;
    const OTHERS: usize = 3_000;
    const PROJECTS: usize = 2_000;
    let home = Home::new();
    let files = actual_history(&home, exit(0, ""));
    // Other threads' rollouts beside the parent's original.
    let day = files[0].parent().unwrap().to_path_buf();
    for index in 0..OTHERS {
        let thread = format!("01c00000-0000-7000-8000-{index:012x}");
        fs::write(
            day.join(format!("rollout-2026-09-07T00-00-00-{thread}.jsonl")),
            b"{}\n",
        )
        .unwrap();
    }
    for index in 0..PROJECTS {
        fs::create_dir_all(home.home.join(format!(".claude/projects/-p{index}"))).unwrap();
    }
    write_child(&home, &child_rows(4203, PROMPT));
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let limits = LaunchLimits {
        max_pass_entries: 50,
        ..LaunchLimits::default()
    };
    let mut backlog = LaunchBacklog::starting();
    // A cancelled pass: nothing looked at, nothing lost.
    let token = CancelToken::new();
    token.cancel();
    let cancelled = continue_claude_launches(
        &mut store,
        &home.home,
        &mut backlog,
        &limits,
        Some(&token),
        T,
    )
    .unwrap();
    assert_eq!((cancelled.entries, cancelled.bytes_read), (0, 0));
    let passes = drain(&home, &mut store, &mut backlog, &limits);
    let per_pass: Vec<u64> = passes.iter().map(|pass| pass.entries).collect();
    assert!(
        per_pass.iter().all(|entries| *entries <= 50),
        "{per_pass:?}"
    );
    let total: u64 = per_pass.iter().sum();
    assert!(
        total >= (OTHERS + PROJECTS) as u64,
        "every entry looked at: {total}"
    );
    assert!(passes.len() >= (OTHERS + PROJECTS) / 50, "{}", passes.len());
    assert_eq!(totals(&passes).linked, 1, "{:?}", totals(&passes));
}

/// A child check whose uniqueness walk is paused over many entry-limited
/// passes; meanwhile the child's first input is rewritten with the same
/// record ID but another text. Nothing is linked from the old match: the
/// check is retried on the new transcript, which no longer matches. An
/// unchanged control links.
#[test]
fn a_child_rewritten_during_its_uniqueness_walk_is_not_linked_from_its_old_text() {
    for rewrite in [false, true] {
        let home = Home::new();
        let files = actual_history(&home, exit(0, ""));
        for index in 0..600 {
            fs::create_dir_all(home.home.join(format!(".claude/projects/-p{index}"))).unwrap();
        }
        let transcript = write_child(&home, &child_rows(4203, PROMPT));
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
        scan_claude(&home, &mut store, &mut SpawnBacklog::default());
        let limits = LaunchLimits {
            max_pass_entries: 20,
            ..LaunchLimits::default()
        };
        let mut backlog = LaunchBacklog::starting();
        // Until the child's transcript was read and its projects are being
        // listed: the group is valid, nothing linked, a pass spent entries.
        let mut walking = 0;
        for _ in 0..10_000 {
            let pass =
                continue_claude_launches(&mut store, &home.home, &mut backlog, &limits, None, T)
                    .unwrap();
            assert_eq!(pass.linked, 0);
            if store.claude_launch_summary().unwrap().groups_valid == 1 && pass.entries == 20 {
                walking += 1;
                if walking == 3 {
                    break;
                }
            }
        }
        assert_eq!(walking, 3, "the walk paused across passes");
        if rewrite {
            let body = fs::read_to_string(&transcript).unwrap();
            let old = serde_json::to_string(PROMPT).unwrap();
            let new = serde_json::to_string(&PROMPT.replace("SYNTHETIC", "REWRITTEN")).unwrap();
            assert!(body.contains(&old));
            fs::write(&transcript, body.replacen(&old, &new, 1)).unwrap();
        }
        let sum = totals(&passes(&home, &mut store, &mut backlog, &limits, 300));
        let child = child_session(&store).unwrap();
        assert_eq!(
            store.claude_launch_creation(&child).unwrap().is_some(),
            !rewrite,
            "rewrite={rewrite}: {sum:?}"
        );
        if rewrite {
            let rows = store
                .claude_launch_candidates_for_child(CHILD, None, 10)
                .unwrap();
            assert_eq!(rows[0].child_state, ChildState::Rejected, "{rows:?}");
        }
    }
}

/// An older native tool output as the Codex app writes it: no call
/// identifier, its own native record identifier, a name and an output.
fn native_output(offset: i64, id: Value, output: Option<Value>) -> Value {
    let mut payload = json!({"type": "function_call_output", "id": id,
        "name": "automation_update", "namespace": "codex_app",
        "internal_chat_message_metadata_passthrough": {}});
    if let Some(output) = output {
        payload["output"] = output;
    }
    json!({"timestamp": iso(offset), "type": "response_item", "payload": payload})
}

fn unpaired(offset: i64) -> Value {
    native_output(
        offset,
        json!("fco_01a082fe-ec0c-70d3-a6d2-f37a59fe4bd3"),
        Some(json!("synthetic update")),
    )
}

/// Native outputs without a call identifier are unpairable, not a broken
/// history: before a launch, in another continuation, or after its
/// acknowledgment, they leave it linkable. Before its acknowledgment they end
/// that launch, and a later independent launch still links. Their own
/// identifier is never a call identifier (neither pairing with nor
/// duplicating a launch's), and a running-cell header they carry makes that
/// cell unusable. Any other malformed tool row still refuses the history.
#[test]
fn native_outputs_without_a_call_id_are_unpairable_not_malformed() {
    let second = "0c000000-0000-4000-8000-0000000000c2";
    let direct = |offset: i64, id: &str, claude: &str, child: &str| {
        vec![
            call(offset, id, &launch_cell(claude, "", child)),
            output(offset + 5, id, DONE, &[exit(0, "")]),
        ]
    };
    let second_child = |home: &Home, first: i64| {
        let project = home.home.join(".claude/projects/-w");
        write_rows(
            &project.join(format!("{second}.jsonl")),
            &[
                json!({"type": "user", "uuid": "0d000000-0000-4000-8000-0000000000b2",
                "parentUuid": null, "timestamp": iso(first), "sessionId": second, "cwd": "/w",
                "message": {"role": "user", "content": PROMPT}}),
            ],
        );
    };
    // (case, [first launch, second launch] linked, history invalid)
    for (case, expected, invalid) in [
        ("before_and_in_another_member", [true, false], false),
        ("before_the_reply", [false, true], false),
        ("after_the_reply", [true, true], false),
        ("own_id_is_the_launch_call_id", [false, false], false),
        ("own_id_is_a_later_launch_call_id", [false, true], false),
        ("ownerless_running_cell", [false, false], false),
        ("ownerless_running_cell_control", [false, true], false),
        ("no_id", [false, false], true),
        ("blank_id", [false, false], true),
        ("number_id", [false, false], true),
        ("no_output", [false, false], true),
        ("call_without_call_id", [false, false], true),
        ("bad_ordinal", [false, false], true),
    ] {
        let home = Home::new();
        let claude = home.claude();
        let mut launch = launch_rows(&claude, CHILD, exit(0, ""));
        // Cut the actual launch's rows before the later resume.
        launch.truncate(13);
        let mut other = vec![message(-3_000_000, "earlier")];
        let mut later = Vec::new();
        match case {
            "before_and_in_another_member" => {
                other.push(unpaired(-2_900_000));
                launch.insert(0, unpaired(-2_000));
            }
            "before_the_reply" | "after_the_reply" => {
                if case == "before_the_reply" {
                    launch.insert(3, unpaired(150));
                } else {
                    launch.insert(5, unpaired(70_000));
                }
                later = direct(3_000_000, "call_later", &claude, second);
            }
            "own_id_is_the_launch_call_id" => launch.insert(
                3,
                native_output(
                    150,
                    json!("call_launch"),
                    Some(json!(exit(0, "").to_string())),
                ),
            ),
            "own_id_is_a_later_launch_call_id" => {
                other.push(native_output(
                    -2_900_000,
                    json!("call_later"),
                    Some(json!("synthetic update")),
                ));
                launch.clear();
                later = direct(3_000_000, "call_later", &claude, second);
            }
            "ownerless_running_cell" | "ownerless_running_cell_control" => {
                launch.clear();
                // Cells are a file's own: the ownerless one in the launch's
                // file, before it.
                if case == "ownerless_running_cell" {
                    launch.push(native_output(
                        2_900_000,
                        json!("fco_01a08ed0-d2df-7710-b05f-cdacb97a037d"),
                        Some(json!("Script running with cell ID 7\n")),
                    ));
                }
                later = vec![
                    call(3_000_000, "call_later", &check_then_launch(&claude, second)),
                    running(3_000_001, "call_later", "7", &[exit(0, "")]),
                    wait(3_000_010, "call_wait_later", "7"),
                    output(3_000_011, "call_wait_later", DONE, &[exit(0, "")]),
                ];
            }
            "no_id" => other.push(native_output(-2_900_000, Value::Null, Some(json!("x")))),
            "blank_id" => other.push(native_output(-2_900_000, json!("  "), Some(json!("x")))),
            "number_id" => other.push(native_output(-2_900_000, json!(7), Some(json!("x")))),
            "no_output" => other.push(native_output(
                -2_900_000,
                json!("fco_01a082fe-ec0c-70d3-a6d2-f37a59fe4bd3"),
                None,
            )),
            "call_without_call_id" => other.push(json!({"timestamp": iso(-2_900_000),
                "type": "response_item", "payload": {"type": "function_call",
                "id": "fc_1", "name": "automation_update", "arguments": "{}"}})),
            _ => {}
        }
        launch.extend(later);
        let files = write_history(
            &home,
            &[vec![message(-3_100_000, "first")], other.clone(), launch],
        );
        if case == "bad_ordinal" {
            // The observed shape with an ordinal out of order still refuses.
            let body = fs::read_to_string(&files[1]).unwrap();
            let mut row = unpaired(-2_900_000);
            row["ordinal"] = json!(99_999);
            fs::write(&files[1], format!("{body}{row}\n")).unwrap();
        }
        write_child(&home, &child_rows(4203, PROMPT));
        second_child(&home, 3_000_002);
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
        scan_claude(&home, &mut store, &mut SpawnBacklog::default());
        let mut backlog = LaunchBacklog::starting();
        let sum = totals(&drain(
            &home,
            &mut store,
            &mut backlog,
            &LaunchLimits::default(),
        ));
        let linked = |native: &str| {
            let child = store
                .user_sessions_with_native(Host::Claude, native)
                .unwrap()
                .into_iter()
                .next()
                .unwrap();
            store.claude_launch_creation(&child).unwrap().is_some()
        };
        let summary = store.claude_launch_summary().unwrap();
        assert_eq!(
            [linked(CHILD), linked(second)],
            expected,
            "{case}: {sum:?} {summary:?}"
        );
        assert_eq!(summary.groups_invalid == 1, invalid, "{case}: {summary:?}");
    }
}

/// Through the ordinary passes: a user input after the child's first input
/// and before the launch's process exits — an interruption marker — leaves
/// the launch linked; one at the first input's own time does not.
#[test]
fn a_later_input_before_the_exit_keeps_the_first_input_unique() {
    for (at, text, linked) in [
        (945_000, "[Request interrupted by user]", true),
        (4203, "Competing input", false),
    ] {
        let home = Home::new();
        let files = actual_history(&home, exit(0, ""));
        let mut rows = child_rows(4203, PROMPT);
        rows.insert(
            2,
            json!({"type": "user", "uuid": "0d000000-0000-4000-8000-0000000000e1",
                "parentUuid": FIRST, "timestamp": iso(at), "sessionId": CHILD, "cwd": "/w",
                "message": {"role": "user", "content": [{"type": "text", "text": text}]}}),
        );
        write_child(&home, &rows);
        let mut store = home.store();
        index_parent(&mut store, &files[0]);
        scan_claude(&home, &mut store, &mut SpawnBacklog::default());
        let mut backlog = LaunchBacklog::starting();
        let sum = totals(&drain(
            &home,
            &mut store,
            &mut backlog,
            &LaunchLimits::default(),
        ));
        let child = child_session(&store).unwrap();
        assert_eq!(
            store.claude_launch_creation(&child).unwrap().is_some(),
            linked,
            "{text}: {sum:?}"
        );
    }
}

/// An output whose emitted results are raw text items, as a tool error or an
/// approval denial is emitted: not a process result.
fn raw_output(offset: i64, id: &str, header: &str, texts: &[&str]) -> Value {
    let mut items = vec![json!({"type": "input_text", "text": header})];
    items.extend(
        texts
            .iter()
            .map(|text| json!({"type": "input_text", "text": text})),
    );
    json!({"timestamp": iso(offset), "type": "response_item", "payload": {
        "type": "custom_tool_call_output", "call_id": id, "output": items}})
}

/// A launch, then a check: the launch is operation 0 of two.
fn launch_then_check(claude: &str, child: &str) -> String {
    format!("{};\n{}", launch_cell(claude, "", child), check_cell())
}

/// The host's own record of a command ending, which this version never
/// reads.
fn exec_end(offset: i64, id: &str, code: i64) -> Value {
    json!({"timestamp": iso(offset), "type": "event_msg", "payload": {
        "type": "exec_command_end", "call_id": id, "exit_code": code}})
}

/// A launch is acknowledged by its own operation's first process result —
/// any exit code, or a running handle — in its own cell's output or the
/// identified continuation of that cell, even while the cell still runs.
/// Nothing after the acknowledgment matters: no later poll, exit, failure,
/// note or host event is needed or can undo it, and the child's first input
/// may come any time after the launch. A tool error, an approval denial, a
/// missing result, another operation's result or another call's output never
/// acknowledges it.
#[test]
fn a_launch_is_acknowledged_by_its_own_first_process_result_alone() {
    let child = ACK_CHILD.to_owned();
    let a = "call_launch_a";
    type Rows = Box<dyn Fn(&str) -> Vec<Value>>;
    let cases: Vec<(&str, i64, bool, Rows)> = vec![
        (
            "exit 0 within the launch, input before it (control)",
            100,
            true,
            Box::new(move |claude| {
                vec![
                    call(0, a, &launch_cell(claude, "", ACK_CHILD)),
                    output(200, a, DONE, &[exit(0, "")]),
                ]
            }),
        ),
        (
            "running, never polled, nothing after",
            500,
            true,
            Box::new(move |claude| {
                vec![
                    call(0, a, &launch_cell(claude, "", ACK_CHILD)),
                    output(200, a, DONE, &[live(4101)]),
                ]
            }),
        ),
        (
            "running, first input long after the reply",
            50_000_000,
            true,
            Box::new(move |claude| {
                vec![
                    call(0, a, &launch_cell(claude, "", ACK_CHILD)),
                    output(200, a, DONE, &[live(4101)]),
                    call(60_000, "call_poll", &poll_cell(4101)),
                    output(60_100, "call_poll", DONE, &[exit(0, "")]),
                ]
            }),
        ),
        (
            "running, the job later fails",
            500,
            true,
            Box::new(move |claude| {
                vec![
                    call(0, a, &launch_cell(claude, "", ACK_CHILD)),
                    output(200, a, DONE, &[live(4101)]),
                    call(60_000, "call_poll", &poll_cell(4101)),
                    output(60_100, "call_poll", DONE, &[exit(1, "failed")]),
                ]
            }),
        ),
        (
            "its own reply is a nonzero exit",
            500,
            true,
            Box::new(move |claude| {
                vec![
                    call(0, a, &launch_cell(claude, "", ACK_CHILD)),
                    output(200, a, DONE, &[exit(2, "error")]),
                ]
            }),
        ),
        (
            "later notes, host events, input and handle reuse",
            500,
            true,
            Box::new(move |claude| {
                vec![
                    call(0, a, &launch_cell(claude, "", ACK_CHILD)),
                    output(200, a, DONE, &[live(4101)]),
                    exec_end(300, a, 1),
                    unpaired(400),
                    message(450, "the worker failed"),
                    call(
                        60_000,
                        "call_input",
                        "text(await tools.write_stdin({session_id: 4101, chars: \"y\\n\"}))",
                    ),
                    output(60_100, "call_input", DONE, &[exit(1, "")]),
                    call(70_000, "call_reuse", &check_cell()),
                    output(70_100, "call_reuse", DONE, &[live(4101)]),
                ]
            }),
        ),
        (
            "the cell yields first; its wait brings the result",
            500,
            true,
            Box::new(move |claude| {
                vec![
                    call(0, a, &check_then_launch(claude, ACK_CHILD)),
                    running(1, a, "9", &[exit(0, "")]),
                    wait(10, "call_wait", "9"),
                    output(11, "call_wait", DONE, &[live(4101)]),
                ]
            }),
        ),
        (
            "the result is emitted while the cell still runs",
            500,
            true,
            Box::new(move |claude| {
                vec![
                    call(0, a, &launch_then_check(claude, ACK_CHILD)),
                    running(200, a, "9", &[live(4101)]),
                ]
            }),
        ),
        (
            "literal assignments lead the command",
            500,
            true,
            Box::new(move |claude| {
                let prefixed = format!("SYNTHETIC_MODE=worker SYNTHETIC_DIR='/w/a b' {claude}");
                vec![
                    call(0, a, &launch_cell(&prefixed, "", ACK_CHILD)),
                    output(200, a, DONE, &[live(4101)]),
                ]
            }),
        ),
        (
            "an assignment that expands",
            500,
            false,
            Box::new(move |claude| {
                let prefixed = format!("SYNTHETIC_DIR=$HOME {claude}");
                vec![
                    call(0, a, &launch_cell(&prefixed, "", ACK_CHILD)),
                    output(200, a, DONE, &[live(4101)]),
                ]
            }),
        ),
        (
            "no reply at all",
            500,
            false,
            Box::new(move |claude| vec![call(0, a, &launch_cell(claude, "", ACK_CHILD))]),
        ),
        (
            "a tool error",
            500,
            false,
            Box::new(move |claude| {
                vec![
                    call(0, a, &launch_cell(claude, "", ACK_CHILD)),
                    raw_output(200, a, DONE, &["exec_command failed: sandbox error"]),
                ]
            }),
        ),
        (
            "an approval denial",
            500,
            false,
            Box::new(move |claude| {
                vec![
                    call(0, a, &launch_cell(claude, "", ACK_CHILD)),
                    output(
                        200,
                        a,
                        DONE,
                        &[json!({"output": "approval denied by the user"})],
                    ),
                ]
            }),
        ),
        (
            "another operation's result",
            500,
            false,
            Box::new(move |claude| {
                vec![
                    call(0, a, &check_then_launch(claude, ACK_CHILD)),
                    raw_output(
                        200,
                        a,
                        DONE,
                        &[&exit(0, "").to_string(), "exec_command failed"],
                    ),
                ]
            }),
        ),
        (
            "the cell completes without the launch's result",
            500,
            false,
            Box::new(move |claude| {
                vec![
                    call(0, a, &check_then_launch(claude, ACK_CHILD)),
                    output(200, a, DONE, &[exit(0, "")]),
                ]
            }),
        ),
        (
            "another call's output",
            500,
            false,
            Box::new(move |claude| {
                vec![
                    call(0, a, &launch_cell(claude, "", ACK_CHILD)),
                    call(100, "call_other", &check_cell()),
                    output(200, "call_other", DONE, &[live(4101)]),
                ]
            }),
        ),
        (
            "a tool error, then another call succeeds",
            500,
            false,
            Box::new(move |claude| {
                vec![
                    call(0, a, &launch_cell(claude, "", ACK_CHILD)),
                    raw_output(200, a, DONE, &["exec_command failed"]),
                    call(300, "call_other", &check_cell()),
                    output(400, "call_other", DONE, &[exit(0, "")]),
                ]
            }),
        ),
    ];
    let mut wrong = Vec::new();
    for (case, first, expected, rows) in cases {
        let home = Home::new();
        let rows = rows(&home.claude());
        let mut store = launch_home(&home, rows, std::slice::from_ref(&child), first);
        let mut backlog = LaunchBacklog::starting();
        let sum = totals(&drain(
            &home,
            &mut store,
            &mut backlog,
            &LaunchLimits::default(),
        ));
        let session = store
            .user_sessions_with_native(Host::Claude, &child)
            .unwrap()
            .into_iter()
            .next()
            .expect("child indexed");
        let proof = store.claude_launch_creation(&session).unwrap();
        if proof.is_some() != expected {
            wrong.push(format!("{case}: linked={} {sum:?}", proof.is_some()));
        }
        if let Some((proof, conflicted)) = proof {
            assert!(!conflicted, "{case}");
            assert_eq!(
                (proof.launch_call_id.as_str(), proof.first_record_uuid.len()),
                (a, 36),
                "{case}"
            );
            assert_eq!(
                shown_parent(&store, &session),
                Some((format!("codex-{PARENT}"), ParentEvidence::AgentLaunch)),
                "{case}"
            );
            assert_eq!(eligible(&home.db, &session), 0, "{case}");
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The one child every case of the acknowledgment tests launches.
const ACK_CHILD: &str = "0c000000-0000-4000-8000-000000000601";

/// A history a previous version validated — current member generations,
/// valid, no launch found: version 1 because the launch never exited,
/// version 3 because its cell also edited a file — is read again once by
/// this version, which links it; then it is not read again.
#[test]
fn a_history_validated_by_the_previous_version_is_read_once_again() {
    for (version, mixed) in [(1, false), (3, true)] {
        a_history_validated_by(version, mixed);
    }
}

fn a_history_validated_by(version: u32, mixed: bool) {
    let home = Home::new();
    let child = ACK_CHILD;
    let rows = if mixed {
        vec![
            call(0, "call_launch_a", &traced_cell(&home.claude(), child)),
            output(
                200,
                "call_launch_a",
                DONE,
                &[json!({}), live(4101), exit(0, "")],
            ),
        ]
    } else {
        vec![
            call(0, "call_launch_a", &launch_cell(&home.claude(), "", child)),
            output(200, "call_launch_a", DONE, &[live(4101)]),
        ]
    };
    let files = write_history(&home, &[vec![message(-3_000_000, "earlier")], rows]);
    let project = home.home.join(".claude/projects/-w");
    fs::create_dir_all(&project).unwrap();
    write_rows(
        &project.join(format!("{child}.jsonl")),
        &[
            json!({"type": "user", "uuid": "0d000000-0000-4000-8000-000000000601",
            "parentUuid": null, "timestamp": iso(500), "sessionId": child, "cwd": "/w",
            "message": {"role": "user", "content": PROMPT}}),
        ],
    );
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    // The previous version's published validation: valid, exactly these
    // members at their generations, no launch.
    {
        let connection = rusqlite::Connection::open(&home.db).unwrap();
        connection
            .execute(
                "INSERT INTO claude_launch_groups VALUES (?1,1,1,'valid',?2)",
                rusqlite::params![PARENT, version],
            )
            .unwrap();
        for (path, rollout) in files.iter().zip([PARENT, ROLLOUTS[0]]) {
            use std::os::unix::fs::MetadataExt;
            let meta = fs::symlink_metadata(path).unwrap();
            connection
                .execute(
                    "INSERT INTO claude_launch_group_members VALUES
                     (?1,?2,1,?3,?4,?5,?6,?7,?5,'valid')",
                    rusqlite::params![
                        PARENT,
                        rollout,
                        meta.dev() as i64,
                        meta.ino() as i64,
                        meta.len() as i64,
                        meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
                        meta.ctime() * 1_000_000_000 + meta.ctime_nsec(),
                    ],
                )
                .unwrap();
        }
    }
    let first = totals(&drain(
        &home,
        &mut store,
        &mut LaunchBacklog::starting(),
        &LaunchLimits::default(),
    ));
    assert_eq!(
        (first.files_read, first.linked, first.threads_unchanged),
        (2, 1, 0),
        "version {version}: {first:?}"
    );
    drop(store);
    let mut store = home.store();
    let again = totals(&drain(
        &home,
        &mut store,
        &mut LaunchBacklog::starting(),
        &LaunchLimits::default(),
    ));
    assert_eq!(
        (again.files_read, again.linked, again.threads_unchanged),
        (0, 0, 1),
        "version {version}: {again:?}"
    );
}

/// A relation the previous version accepted for this launch — the same
/// child, parent, first input, launch operation, file and launch ordinal,
/// with its old completion anchor and version — is kept exactly as stored:
/// the new proof replays it and the launch is linked. The Human view is
/// unchanged by it.
#[test]
fn a_relation_the_previous_version_accepted_is_kept_byte_for_byte() {
    let home = Home::new();
    let files = actual_history(&home, exit(0, ""));
    write_child(&home, &child_rows(4203, PROMPT));
    let mut store = home.store();
    index_parent(&mut store, &files[0]);
    scan_claude(&home, &mut store, &mut SpawnBacklog::default());
    let child = child_session(&store).unwrap();
    let ordinal = |id: &str, kind: &str| -> i64 {
        let body = fs::read_to_string(files.last().unwrap()).unwrap();
        body.lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|row| row["payload"]["call_id"] == id && row["payload"]["type"] == kind)
            .unwrap()["ordinal"]
            .as_i64()
            .unwrap()
    };
    let launch_ordinal = ordinal("call_launch", "custom_tool_call");
    let completion_ordinal = ordinal("call_completion", "custom_tool_call_output");
    let connection = rusqlite::Connection::open(&home.db).unwrap();
    connection
        .execute(
            "INSERT INTO session_creation_relations(child_session_id,child_host,
                 child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
                 evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
                 launch_call_id,launch_operation_index,process_session_id,completion_call_id,
                 completion_operation_index,segment_rollout_id,launch_ordinal,completion_ordinal)
             VALUES (?1,'claude',?2,'codex',?3,'codex_claude_launch',1,
                 'codex_exec_session_id_launch','accepted',?4,?5,?6,'call_launch',0,'56751',
                 'call_completion',0,?7,?8,?9)",
            rusqlite::params![
                child,
                CHILD,
                PARENT,
                T - 1,
                format!("codex-{PARENT}"),
                FIRST,
                ROLLOUTS[2],
                launch_ordinal,
                completion_ordinal
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cli_artifact_launch_owners VALUES (?1,'call_launch',0,?2,0)",
            rusqlite::params![format!("codex-{PARENT}"), child],
        )
        .unwrap();
    let dump = |connection: &rusqlite::Connection| -> String {
        connection
            .query_row(
                "SELECT quote(child_session_id)||quote(child_native_session_id)||
                     quote(parent_native_session_id)||quote(evidence_kind)||
                     quote(evidence_version)||quote(witness)||quote(state)||quote(recorded_at)||
                     quote(parent_session_id)||quote(first_record_uuid)||quote(launch_call_id)||
                     quote(launch_operation_index)||quote(process_session_id)||
                     quote(completion_call_id)||quote(output_read_call_id)||
                     quote(provider_result_uuid)||quote(completion_operation_index)||
                     quote(segment_rollout_id)||quote(launch_ordinal)||quote(completion_ordinal)
                 FROM session_creation_relations",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    let before = dump(&connection);
    let human = eligible(&home.db, &child);
    let total = totals(&settle(&home, &mut store, &mut SpawnBacklog::starting()));
    assert_eq!((total.linked, total.changed), (1, 0), "{total:?}");
    assert_eq!(dump(&connection), before, "the accepted row is untouched");
    assert_eq!(eligible(&home.db, &child), human);
    let (proof, conflicted) = store.claude_launch_creation(&child).unwrap().unwrap();
    assert!(!conflicted);
    assert_eq!(
        (proof.evidence_version, proof.launch_ordinal),
        (1, Some(launch_ordinal))
    );
    let rows = store
        .claude_launch_candidates_for_child(CHILD, None, 10)
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].child_state, ChildState::Linked);
}

/// An unrelated file edit, an opaque operation of a cell. Its patch quotes a
/// launch of `child` and a process number, which are never read.
fn patch_op(claude: &str, child: &str) -> String {
    format!(
        "text(await tools.apply_patch({}))",
        js(&format!(
            "*** Begin Patch\n*** Update File: /w/plan.md\n+Started `{claude} -p --session-id {child} \
             --output-format json 'brief'` as process 4101.\n*** End Patch\n"
        ))
    )
}

/// An unrelated plan update with nested plain-data arguments, bound.
fn plan_op() -> String {
    "const u = await tools.update_plan({plan: [{step: \"launch\", status: \"done\"}], \
     note: null, n: 4101});\ntext(u)"
        .into()
}

/// The traced shape: a file edit, the launch, then a bookkeeping command.
fn traced_cell(claude: &str, child: &str) -> String {
    format!(
        "{};\n{};\n{}",
        patch_op(claude, child),
        launch_cell(claude, "", child),
        check_cell()
    )
}

/// A launch shares its cell with other tools: it is selected by its own
/// position and acknowledged by its own result, whatever the other
/// operations are, wherever they stand and whatever their arguments quote.
/// A result out of place, a missing, extra or unemitted one, a reused call
/// identifier, a launch only quoted in another tool's arguments, or another
/// tool's arguments that are computed or run conditionally link nothing.
#[test]
fn a_launch_among_other_tools_in_one_cell_is_selected_by_its_own_position() {
    let a = "call_launch_a";
    type Rows = Box<dyn Fn(&str) -> Vec<Value>>;
    let cases: Vec<(&str, bool, Rows)> = vec![
        (
            "edit, launch, bookkeeping",
            true,
            Box::new(move |claude| {
                vec![
                    call(0, a, &traced_cell(claude, ACK_CHILD)),
                    output(200, a, DONE, &[json!({}), live(4101), exit(0, "")]),
                ]
            }),
        ),
        (
            "launch, bound plan update, template-literal edit",
            true,
            Box::new(move |claude| {
                let cell = format!(
                    "{};\n{};\ntext(await tools.apply_patch(`*** Begin Patch\n+a \\` b $c {{d}}\n*** End Patch`))",
                    launch_cell(claude, "", ACK_CHILD),
                    plan_op()
                );
                vec![
                    call(0, a, &cell),
                    output(200, a, DONE, &[live(4101), json!({}), json!({})]),
                ]
            }),
        ),
        (
            "check, edit, launch: the cell yields; its wait brings the result",
            true,
            Box::new(move |claude| {
                let cell = format!(
                    "{};\n{};\n{}",
                    check_cell(),
                    patch_op(claude, ACK_CHILD),
                    launch_cell(claude, "", ACK_CHILD)
                );
                vec![
                    call(0, a, &cell),
                    running(1, a, "9", &[exit(0, ""), json!({})]),
                    wait(10, "call_wait", "9"),
                    output(11, "call_wait", DONE, &[live(4101)]),
                ]
            }),
        ),
        (
            "launch, then an edit still running",
            true,
            Box::new(move |claude| {
                let cell = format!(
                    "{};\n{}",
                    launch_cell(claude, "", ACK_CHILD),
                    patch_op(claude, ACK_CHILD)
                );
                vec![call(0, a, &cell), running(200, a, "9", &[live(4101)])]
            }),
        ),
        (
            "a plan update with signed, decimal and exponent numbers",
            true,
            Box::new(move |claude| {
                let cell = format!(
                    "{};\n{};\ntext(await tools.update_plan(-1, {{n: 1.5, z: -0, t: 1e-3, l: [-2, 2.5E+3, {{k: -0.25}}]}}))",
                    patch_op(claude, ACK_CHILD),
                    launch_cell(claude, "", ACK_CHILD)
                );
                vec![
                    call(0, a, &cell),
                    output(200, a, DONE, &[json!({}), live(4101), json!({})]),
                ]
            }),
        ),
        (
            "a calculation in the plan update",
            false,
            Box::new(move |claude| {
                let cell = format!(
                    "{};\ntext(await tools.update_plan({{n: 1 + 2}}))",
                    launch_cell(claude, "", ACK_CHILD)
                );
                vec![
                    call(0, a, &cell),
                    output(200, a, DONE, &[live(4101), json!({})]),
                ]
            }),
        ),
        (
            "a malformed number in the plan update",
            false,
            Box::new(move |claude| {
                let cell = format!(
                    "{};\ntext(await tools.update_plan({{n: 01.5, h: 0x1F}}))",
                    launch_cell(claude, "", ACK_CHILD)
                );
                vec![
                    call(0, a, &cell),
                    output(200, a, DONE, &[live(4101), json!({})]),
                ]
            }),
        ),
        (
            "the launch's own wait time as a decimal",
            false,
            Box::new(move |claude| {
                let cell = traced_cell(claude, ACK_CHILD)
                    .replace("yield_time_ms: 10000", "yield_time_ms: 1e4");
                vec![
                    call(0, a, &cell),
                    output(200, a, DONE, &[json!({}), live(4101), exit(0, "")]),
                ]
            }),
        ),
        (
            "the launch's own wait time negative",
            false,
            Box::new(move |claude| {
                let cell = traced_cell(claude, ACK_CHILD)
                    .replace("yield_time_ms: 10000", "yield_time_ms: -1");
                vec![
                    call(0, a, &cell),
                    output(200, a, DONE, &[json!({}), live(4101), exit(0, "")]),
                ]
            }),
        ),
        (
            "the edit's result in the launch's place",
            false,
            Box::new(move |claude| {
                vec![
                    call(0, a, &traced_cell(claude, ACK_CHILD)),
                    output(200, a, DONE, &[live(4101), json!({}), exit(0, "")]),
                ]
            }),
        ),
        (
            "one result too few",
            false,
            Box::new(move |claude| {
                vec![
                    call(0, a, &traced_cell(claude, ACK_CHILD)),
                    output(200, a, DONE, &[json!({}), live(4101)]),
                ]
            }),
        ),
        (
            "one result too many",
            false,
            Box::new(move |claude| {
                vec![
                    call(0, a, &traced_cell(claude, ACK_CHILD)),
                    output(
                        200,
                        a,
                        DONE,
                        &[json!({}), live(4101), exit(0, ""), exit(0, "")],
                    ),
                ]
            }),
        ),
        (
            "the call identifier used again",
            false,
            Box::new(move |claude| {
                vec![
                    call(0, a, &traced_cell(claude, ACK_CHILD)),
                    output(200, a, DONE, &[json!({}), live(4101), exit(0, "")]),
                    call(300, a, &check_cell()),
                    output(400, a, DONE, &[exit(0, "")]),
                ]
            }),
        ),
        (
            "the launch only quoted in the edit",
            false,
            Box::new(move |claude| {
                let cell = format!("{};\n{}", patch_op(claude, ACK_CHILD), check_cell());
                vec![
                    call(0, a, &cell),
                    output(200, a, DONE, &[json!({}), live(4101)]),
                ]
            }),
        ),
        (
            "the edit's text substituted",
            false,
            Box::new(move |claude| {
                let cell = traced_cell(claude, ACK_CHILD).replacen(
                    "text(await tools.apply_patch(",
                    "text(await tools.apply_patch(`${p}` + ",
                    1,
                );
                vec![
                    call(0, a, &cell),
                    output(200, a, DONE, &[json!({}), live(4101), exit(0, "")]),
                ]
            }),
        ),
        (
            "the edit run conditionally",
            false,
            Box::new(move |claude| {
                let cell = format!("if (ok) {}", traced_cell(claude, ACK_CHILD));
                vec![
                    call(0, a, &cell),
                    output(200, a, DONE, &[json!({}), live(4101), exit(0, "")]),
                ]
            }),
        ),
        (
            "the edit's result never emitted",
            false,
            Box::new(move |claude| {
                let cell = traced_cell(claude, ACK_CHILD).replacen(
                    "text(await tools.apply_patch(",
                    "await (tools.apply_patch(",
                    1,
                );
                vec![
                    call(0, a, &cell),
                    output(200, a, DONE, &[live(4101), exit(0, "")]),
                ]
            }),
        ),
    ];
    let child = ACK_CHILD.to_owned();
    let mut wrong = Vec::new();
    for (case, expected, rows) in cases {
        let home = Home::new();
        let rows = rows(&home.claude());
        let mut store = launch_home(&home, rows, std::slice::from_ref(&child), 500);
        let sum = totals(&drain(
            &home,
            &mut store,
            &mut LaunchBacklog::starting(),
            &LaunchLimits::default(),
        ));
        let session = store
            .user_sessions_with_native(Host::Claude, &child)
            .unwrap()
            .into_iter()
            .next()
            .expect("child indexed");
        let proof = store.claude_launch_creation(&session).unwrap();
        if proof.is_some() != expected {
            wrong.push(format!("{case}: linked={} {sum:?}", proof.is_some()));
        }
        if let Some((proof, conflicted)) = proof {
            assert!(!conflicted, "{case}");
            assert_eq!(proof.launch_call_id, a, "{case}");
            assert_eq!(
                shown_parent(&store, &session),
                Some((format!("codex-{PARENT}"), ParentEvidence::AgentLaunch)),
                "{case}"
            );
            assert_eq!(eligible(&home.db, &session), 0, "{case}");
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Two launches in one cell between other tools each take their own
/// position's result; a result out of place breaks only its own launch.
#[test]
fn launches_between_other_tools_each_take_their_own_result() {
    let children: Vec<String> = (0..2)
        .map(|index| format!("0c000000-0000-4000-8000-{:012x}", 0x700 + index))
        .collect();
    for (case, results, expected) in [
        (
            "in place",
            vec![json!({}), live(4101), json!({}), live(4102), exit(0, "")],
            2,
        ),
        (
            "the plan's result before the first launch's",
            vec![json!({}), json!({}), live(4101), live(4102), exit(0, "")],
            1,
        ),
    ] {
        let home = Home::new();
        let claude = home.claude();
        // Edit 0, launch 1, plan 2, launch 3, check 4.
        let cell = format!(
            "{};\n{}{}",
            patch_op(&claude, &children[0]),
            launches_cell(&claude, &children).replacen(
                "text(r0);\n",
                &format!("text(r0);\n{};\n", plan_op()),
                1
            ),
            check_cell()
        );
        let rows = vec![
            call(0, "call_mixed", &cell),
            output(200, "call_mixed", DONE, &results),
        ];
        let mut store = launch_home(&home, rows, &children, 500);
        let sum = totals(&drain(
            &home,
            &mut store,
            &mut LaunchBacklog::starting(),
            &LaunchLimits::default(),
        ));
        assert_eq!(
            store.claude_launch_summary().unwrap().relations_accepted,
            expected,
            "{case}: {sum:?}"
        );
        if expected == 1 {
            let second = store
                .user_sessions_with_native(Host::Claude, &children[1])
                .unwrap()
                .into_iter()
                .next()
                .expect("child indexed");
            let (proof, _) = store.claude_launch_creation(&second).unwrap().unwrap();
            assert_eq!(proof.launch_operation_index, 3, "{case}");
        }
    }
}

/// A history this version reads again because an earlier version validated
/// it keeps every relation already accepted exactly as stored, links nothing
/// twice, and is not read again while unchanged.
#[test]
fn reading_a_history_again_for_a_new_version_keeps_accepted_relations() {
    let home = Home::new();
    let child = ACK_CHILD;
    let rows = vec![
        call(0, "call_launch_a", &traced_cell(&home.claude(), child)),
        output(
            200,
            "call_launch_a",
            DONE,
            &[json!({}), live(4101), exit(0, "")],
        ),
    ];
    let mut store = launch_home(&home, rows, &[child.to_owned()], 500);
    let run = |store: &mut Store| {
        totals(&drain(
            &home,
            store,
            &mut LaunchBacklog::starting(),
            &LaunchLimits::default(),
        ))
    };
    let first = run(&mut store);
    assert_eq!(first.linked, 1, "{first:?}");
    let connection = rusqlite::Connection::open(&home.db).unwrap();
    let dump = || -> Vec<String> {
        let mut statement = connection
            .prepare("SELECT quote(evidence_version)||quote(state)||quote(recorded_at)||quote(launch_operation_index)||quote(launch_call_id)||quote(completion_call_id)||quote(completion_operation_index)||quote(process_session_id) FROM session_creation_relations ORDER BY child_session_id")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let before = dump();
    assert_eq!(before.len(), 1);
    connection
        .execute(
            "UPDATE claude_launch_groups SET validator_version=3 WHERE parent_native_session_id=?1",
            [PARENT],
        )
        .unwrap();
    drop(store);
    let mut store = home.store();
    let again = run(&mut store);
    assert_eq!(
        (again.files_read, again.changed, again.threads_unchanged),
        (2, 0, 0),
        "{again:?}"
    );
    assert_eq!(dump(), before, "the accepted row is untouched");
    assert_eq!(store.claude_launch_summary().unwrap().relations_accepted, 1);
    drop(store);
    let mut store = home.store();
    let warm = run(&mut store);
    assert_eq!(
        (
            warm.files_read,
            warm.linked,
            warm.changed,
            warm.threads_unchanged
        ),
        (0, 0, 0, 1),
        "{warm:?}"
    );
    assert_eq!(dump(), before);
}
