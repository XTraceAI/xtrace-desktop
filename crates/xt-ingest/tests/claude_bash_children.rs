//! Child facts for Claude sessions a Claude agent launched from its own
//! `Bash` tool with no session identifier: a finite loop over two literal
//! folders running `claude -p` with one literal prompt and printing each
//! plain answer. The two children are found by their exact first input,
//! folder, birth within the call and whole first answer, each the only
//! possible child of its launch, and recorded as children with no parent.
//!
//! Every file is written here: a synthetic caller transcript and the
//! launched sessions' transcripts. Nothing real is read and no command runs.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use xt_ingest::native::{
    ImportRequest, ProducerSource,
    claude_launch::{
        LaunchLimits,
        bash::{BashBacklog, BashProgress, continue_claude_bash},
    },
    readers_cli::read_pin,
    session_creation::{SpawnBacklog, continue_codex_spawns, spawn_limits},
};
use xt_store::{Host, Store, child_fact::ChildEvidence, session_list};

const CALLER: &str = "0c000000-0000-4000-8000-0000000000ca";
const ALPHA: &str = "0c000000-0000-4000-8000-0000000000a1";
const BETA: &str = "0c000000-0000-4000-8000-0000000000b1";
const OTHER: &str = "0c000000-0000-4000-8000-0000000000e1";
const SECOND_CALLER: &str = "0c000000-0000-4000-8000-0000000000cb";
const NEIGHBOUR: &str = "0c000000-0000-4000-8000-0000000000e2";
const QUESTION: &str = "Synthetic install-rule question?";
const CALL: &str = "toolu_01SyntheticLoop";
/// The Bash call's time; everything else is an offset from it.
const AT: i64 = 1_790_668_716_333;
const T: i64 = 1_790_700_000_000;

fn iso(offset: i64) -> String {
    chrono::DateTime::from_timestamp_millis(AT + offset)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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

    fn project(&self, folder: &str) -> PathBuf {
        let path = self.home.join(".claude/projects").join(folder);
        fs::create_dir_all(&path).unwrap();
        path
    }
}

fn write_rows(path: &Path, rows: &[Value]) {
    let body: String = rows.iter().map(|row| format!("{row}\n")).collect();
    fs::write(path, body).unwrap();
}

/// The loop the trace records: an assigned prompt, two literal folders, a
/// marker line before each plain answer.
fn command() -> String {
    format!(
        "Q=\"{QUESTION}\"; for d in /w/alpha /w/beta; do echo \"== $d\"; \
         (cd \"$d\" && claude -p \"$Q\" --max-turns 1 2>&1 | tail -4); done"
    )
}

fn printed(alpha: &str, beta: &str) -> String {
    format!("== /w/alpha\n{alpha}\n== /w/beta\n{beta}\n")
}

/// A caller transcript: a person's request, the Bash call and its own
/// complete, clean result.
fn caller_rows(native: &str, call: &str, result_offset: i64, stdout: &str) -> Vec<Value> {
    let line = |kind: &str, uuid: &str, offset: i64, message: Value| {
        json!({"type": kind, "uuid": uuid, "parentUuid": null, "timestamp": iso(offset),
            "sessionId": native, "isSidechain": false, "cwd": "/w/caller",
            "entrypoint": "cli", "message": message})
    };
    let mut result = line(
        "user",
        &format!("{native}-result"),
        result_offset,
        json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": call, "content": stdout}]}),
    );
    result["toolUseResult"] =
        json!({"stdout": stdout, "stderr": "", "interrupted": false, "isImage": false});
    vec![
        line(
            "user",
            &format!("{native}-ask"),
            -60_000,
            json!({"role": "user", "content": "Ask two fresh sessions"}),
        ),
        line(
            "assistant",
            &format!("{native}-call"),
            0,
            json!({"role": "assistant", "id": format!("msg-{native}"),
                "model": "fixture-model",
                "content": [{"type": "tool_use", "id": call, "name": "Bash",
                    "input": {"command": command(), "description": "Ask fresh sessions"}}],
                "usage": {"input_tokens": 3, "output_tokens": 2}}),
        ),
        result,
    ]
}

/// A launched session: its first input the prompt, in `cwd`, at `born`, and
/// its first turn's one answer.
fn child_rows(native: &str, cwd: &str, born: i64, question: &str, answer: &str) -> Vec<Value> {
    vec![
        json!({"type": "user", "uuid": format!("{native}-first"), "parentUuid": null,
            "timestamp": iso(born), "sessionId": native, "cwd": cwd,
            "entrypoint": "sdk-cli", "promptSource": "sdk",
            "message": {"role": "user", "content": question}}),
        json!({"type": "assistant", "uuid": format!("{native}-answer"),
            "parentUuid": format!("{native}-first"), "timestamp": iso(born + 2000),
            "sessionId": native, "cwd": cwd, "entrypoint": "sdk-cli",
            "message": {"role": "assistant", "id": format!("msg-{native}"),
                "model": "fixture-model", "content": [{"type": "text", "text": answer}],
                "usage": {"input_tokens": 3, "output_tokens": 2}}}),
    ]
}

/// The two children of the loop, the caller, and nothing else.
fn standard(home: &Home) {
    write_rows(
        &home.project("-w-caller").join(format!("{CALLER}.jsonl")),
        &caller_rows(
            CALLER,
            CALL,
            12_000,
            &printed("Answer alpha", "Answer beta"),
        ),
    );
    write_rows(
        &home.project("-w-alpha").join(format!("{ALPHA}.jsonl")),
        &child_rows(ALPHA, "/w/alpha", 3_000, QUESTION, "Answer alpha"),
    );
    write_rows(
        &home.project("-w-beta").join(format!("{BETA}.jsonl")),
        &child_rows(BETA, "/w/beta", 8_000, QUESTION, "Answer beta"),
    );
}

/// Index the Claude host through the ordinary importer.
fn scan(home: &Home, store: &mut Store, spawns: &mut SpawnBacklog) {
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

/// Bounded passes until nothing is pending; their counts summed.
fn settle(home: &Home, store: &mut Store, backlog: &mut BashBacklog) -> BashProgress {
    settle_with(home, store, backlog, &LaunchLimits::default(), 1_000)
}

/// [`settle`] with these pass limits, in at most `passes` passes.
fn settle_with(
    home: &Home,
    store: &mut Store,
    backlog: &mut BashBacklog,
    limits: &LaunchLimits,
    passes: usize,
) -> BashProgress {
    let mut total = BashProgress::default();
    for _ in 0..passes {
        if !backlog.pending() {
            return total;
        }
        let pass = continue_claude_bash(store, &home.home, backlog, limits, None, T).unwrap();
        total.callers_read += pass.callers_read;
        total.launches += pass.launches;
        total.children += pass.children;
        total.already += pass.already;
        total.ambiguous += pass.ambiguous;
        total.rejected += pass.rejected;
        total.waiting += pass.waiting;
        total.changed += pass.changed;
    }
    panic!("never settled: {total:?}");
}

fn session(store: &Store, native: &str) -> String {
    store
        .user_sessions_with_native(Host::Claude, native)
        .unwrap()
        .remove(0)
}

/// `(kind, source, call, position, accepted)` of one stored fact.
type Stored = (ChildEvidence, String, Option<String>, Option<u32>, bool);

/// Every stored fact of one session, as [`Stored`].
fn facts(store: &Store, native: &str) -> Vec<Stored> {
    store
        .child_facts(&session(store, native))
        .unwrap()
        .into_iter()
        .map(|(fact, accepted)| {
            (
                fact.evidence_kind,
                fact.source_native_session_id,
                fact.launch_call_id,
                fact.launch_operation_index,
                accepted,
            )
        })
        .collect()
}

fn bash(source: &str, call: &str, index: u32) -> Stored {
    (
        ChildEvidence::ClaudeBashLaunch,
        source.to_owned(),
        Some(call.to_owned()),
        Some(index),
        true,
    )
}

fn known(home: &Home, store: &Store, native: &str) -> (bool, bool) {
    let connection = rusqlite::Connection::open(&home.db).unwrap();
    let context = session_list::context(&connection, &[&session(store, native)])
        .unwrap()
        .remove(0);
    (context.known_child, context.parent.is_some())
}

/// Human-eligible input records of a session, as the app's hours read them.
fn eligible(home: &Home, store: &Store, native: &str) -> i64 {
    rusqlite::Connection::open(&home.db)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM v_human_inputs h JOIN records r ON r.uuid=h.uuid
             WHERE r.session_id=?1 AND h.human_is_eligible=1",
            [session(store, native)],
            |row| row.get(0),
        )
        .unwrap()
}

/// Every file under the home, with its bytes.
fn snapshot(home: &Home) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![home.home.clone()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn both_loop_children_are_known_children_without_a_parent() {
    let home = Home::new();
    standard(&home);
    let mut store = home.store();
    scan(&home, &mut store, &mut SpawnBacklog::default());
    let human = [ALPHA, BETA].map(|native| eligible(&home, &store, native));
    let before = snapshot(&home);
    let total = settle(&home, &mut store, &mut BashBacklog::starting());
    assert_eq!((total.callers_read, total.launches), (1, 2), "{total:?}");
    assert_eq!((total.children, total.changed), (2, 2), "{total:?}");
    assert_eq!(facts(&store, ALPHA), [bash(CALLER, CALL, 0)]);
    assert_eq!(facts(&store, BETA), [bash(CALLER, CALL, 1)]);
    for (native, human) in [ALPHA, BETA].iter().zip(human) {
        // Known, no parent, and the Human view untouched.
        assert_eq!(known(&home, &store, native), (true, false), "{native}");
        assert!(
            store
                .session_creation(&session(&store, native))
                .unwrap()
                .is_none()
        );
        assert_eq!(eligible(&home, &store, native), human, "{native}");
    }
    // The caller is no child.
    assert_eq!(known(&home, &store, CALLER), (false, false));
    assert_eq!(snapshot(&home), before, "no host file written");
    // A restart sweeps again and changes nothing.
    let again = settle(&home, &mut store, &mut BashBacklog::starting());
    assert_eq!((again.changed, again.children), (0, 0), "{again:?}");
    assert_eq!(again.already, 2, "{again:?}");
    assert_eq!(store.child_fact_summary().unwrap().children, 2);
}

/// The same loop launched with streaming JSON or verbose output is read as
/// a call of its own format, but what it printed is never matched as an
/// answer: even printed text equal to each child's answer decides no child.
/// The loop with neutral options of the shared map, in their `=` spelling or
/// not, decides both children as before.
#[test]
fn a_streaming_or_verbose_loop_decides_no_child_and_neutral_options_change_nothing() {
    for (options, children) in [
        ("--max-turns 1 --verbose --output-format stream-json", 0),
        ("--max-turns 1 --verbose", 0),
        ("--max-turns 1 --output-format json --verbose", 0),
        (
            "--max-turns=1 --no-chrome --name t --tools Read --output-format=text",
            2,
        ),
    ] {
        let home = Home::new();
        standard(&home);
        let mut rows = caller_rows(
            CALLER,
            CALL,
            12_000,
            &printed("Answer alpha", "Answer beta"),
        );
        let edited = command().replace("--max-turns 1", options);
        assert_ne!(edited, command());
        rows[1]["message"]["content"][0]["input"]["command"] = json!(edited);
        write_rows(
            &home.project("-w-caller").join(format!("{CALLER}.jsonl")),
            &rows,
        );
        let mut store = home.store();
        scan(&home, &mut store, &mut SpawnBacklog::default());
        let total = settle(&home, &mut store, &mut BashBacklog::starting());
        assert_eq!(total.callers_read, 1, "{options}: {total:?}");
        assert_eq!(
            (total.launches, total.children),
            (children, children),
            "{options}: {total:?}"
        );
        assert_eq!(
            facts(&store, ALPHA).len() + facts(&store, BETA).len(),
            children,
            "{options}"
        );
    }
}

/// A second possible child of one launch, an unreadable history among the
/// launch's possible children, an answer that is not the printed one, or a
/// first input that is not the prompt each leaves that launch's child
/// unknown; the other launch is decided as before. A second possible child
/// that appears later withholds the fact already resting on its launch.
#[test]
fn only_the_one_exact_possible_child_is_known() {
    type Edit = Box<dyn Fn(&Home)>;
    let cases: Vec<(&str, Edit)> = vec![
        (
            "a second possible child",
            Box::new(|home: &Home| {
                write_rows(
                    &home.project("-w-alpha").join(format!("{OTHER}.jsonl")),
                    &child_rows(OTHER, "/w/alpha", 4_000, QUESTION, "Answer alpha"),
                );
            }),
        ),
        (
            "an unreadable history beside it",
            Box::new(|home: &Home| {
                fs::write(
                    home.project("-w-alpha").join(format!("{OTHER}.jsonl")),
                    "{\"type\":\"user\",\"sessionId\":",
                )
                .unwrap();
            }),
        ),
        (
            "another answer",
            Box::new(|home: &Home| {
                write_rows(
                    &home.project("-w-alpha").join(format!("{ALPHA}.jsonl")),
                    &child_rows(ALPHA, "/w/alpha", 3_000, QUESTION, "Something else"),
                );
            }),
        ),
        (
            "another first input",
            Box::new(|home: &Home| {
                write_rows(
                    &home.project("-w-alpha").join(format!("{ALPHA}.jsonl")),
                    &child_rows(ALPHA, "/w/alpha", 3_000, "Another question", "Answer alpha"),
                );
            }),
        ),
        (
            "born after the result",
            Box::new(|home: &Home| {
                write_rows(
                    &home.project("-w-alpha").join(format!("{ALPHA}.jsonl")),
                    &child_rows(ALPHA, "/w/alpha", 20_000, QUESTION, "Answer alpha"),
                );
            }),
        ),
    ];
    for (name, edit) in cases {
        let home = Home::new();
        standard(&home);
        edit(&home);
        let mut store = home.store();
        scan(&home, &mut store, &mut SpawnBacklog::default());
        let total = settle(&home, &mut store, &mut BashBacklog::starting());
        assert!(facts(&store, ALPHA).is_empty(), "{name}: {total:?}");
        assert_eq!(known(&home, &store, ALPHA), (false, false), "{name}");
        assert_eq!(facts(&store, BETA), [bash(CALLER, CALL, 1)], "{name}");
    }
    // Recorded first; a second possible child appearing afterwards withholds
    // that launch's fact, and only it.
    let home = Home::new();
    standard(&home);
    let mut store = home.store();
    scan(&home, &mut store, &mut SpawnBacklog::default());
    settle(&home, &mut store, &mut BashBacklog::starting());
    assert_eq!(known(&home, &store, ALPHA), (true, false));
    write_rows(
        &home.project("-w-alpha").join(format!("{OTHER}.jsonl")),
        &child_rows(OTHER, "/w/alpha", 4_000, QUESTION, "Answer alpha"),
    );
    let after = settle(&home, &mut store, &mut BashBacklog::starting());
    assert_eq!((after.ambiguous, after.changed), (1, 1), "{after:?}");
    assert_eq!(known(&home, &store, ALPHA), (false, false));
    assert_eq!(known(&home, &store, BETA), (true, false));
    // For good: the second history removed, the launch stays withheld.
    fs::remove_file(home.project("-w-alpha").join(format!("{OTHER}.jsonl"))).unwrap();
    let removed = settle(&home, &mut store, &mut BashBacklog::starting());
    assert_eq!(removed.changed, 0, "{removed:?}");
    assert_eq!(known(&home, &store, ALPHA), (false, false));
}

/// Another session holding the very same call and result is another
/// possible creator: only the parent is uncertain, and each launch's fact is
/// kept beside the other's.
#[test]
fn a_competing_caller_leaves_the_children_known() {
    let home = Home::new();
    standard(&home);
    write_rows(
        &home
            .project("-w-caller")
            .join(format!("{SECOND_CALLER}.jsonl")),
        &caller_rows(
            SECOND_CALLER,
            "toolu_01SecondLoop",
            12_000,
            &printed("Answer alpha", "Answer beta"),
        ),
    );
    let mut store = home.store();
    scan(&home, &mut store, &mut SpawnBacklog::default());
    let total = settle(&home, &mut store, &mut BashBacklog::starting());
    assert_eq!(total.children, 4, "{total:?}");
    let mut alpha = facts(&store, ALPHA);
    alpha.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(
        alpha,
        [
            bash(CALLER, CALL, 0),
            bash(SECOND_CALLER, "toolu_01SecondLoop", 0)
        ]
    );
    assert_eq!(known(&home, &store, ALPHA), (true, false));
    assert_eq!(known(&home, &store, BETA), (true, false));
}

/// A child imported after its caller was read is found when it is: the
/// ordinary scan names it, its caller's launch waits for it, and a pass then
/// records it without reading the caller again. A caller read before its
/// call had a result is read again once it has one.
#[test]
fn children_and_results_that_arrive_later_are_found() {
    let home = Home::new();
    // The caller only, its call still open.
    let mut open = caller_rows(
        CALLER,
        CALL,
        12_000,
        &printed("Answer alpha", "Answer beta"),
    );
    open.pop();
    write_rows(
        &home.project("-w-caller").join(format!("{CALLER}.jsonl")),
        &open,
    );
    let mut store = home.store();
    let mut spawns = SpawnBacklog::starting();
    scan(&home, &mut store, &mut spawns);
    let first = settle(&home, &mut store, &mut spawns.bash);
    assert_eq!((first.callers_read, first.launches), (1, 0), "{first:?}");
    // The result and one child arrive.
    standard(&home);
    fs::remove_file(home.project("-w-beta").join(format!("{BETA}.jsonl"))).unwrap();
    scan(&home, &mut store, &mut spawns);
    let second = settle(&home, &mut store, &mut spawns.bash);
    assert_eq!(second.callers_read, 1, "{second:?}");
    assert_eq!(facts(&store, ALPHA), [bash(CALLER, CALL, 0)]);
    // The other child arrives: decided without reading the caller again.
    write_rows(
        &home.project("-w-beta").join(format!("{BETA}.jsonl")),
        &child_rows(BETA, "/w/beta", 8_000, QUESTION, "Answer beta"),
    );
    scan(&home, &mut store, &mut spawns);
    let third = settle(&home, &mut store, &mut spawns.bash);
    assert_eq!(third.callers_read, 0, "{third:?}");
    assert_eq!(facts(&store, BETA), [bash(CALLER, CALL, 1)]);
}

/// The watcher's own background pass runs the sweep: a backlog it starts
/// with records both children without any other call.
#[test]
fn the_watcher_pass_records_them() {
    let home = Home::new();
    standard(&home);
    let mut store = home.store();
    scan(&home, &mut store, &mut SpawnBacklog::default());
    let mut spawns = SpawnBacklog::starting();
    let mut children = 0;
    for _ in 0..1_000 {
        if !spawns.pending() {
            break;
        }
        let pass =
            continue_codex_spawns(&mut store, &home.home, &mut spawns, spawn_limits(), None, T)
                .unwrap();
        children += pass.bash.map_or(0, |bash| bash.children);
    }
    assert!(!spawns.pending());
    assert_eq!(children, 2);
    assert_eq!(known(&home, &store, ALPHA), (true, false));
    assert_eq!(known(&home, &store, BETA), (true, false));
}

/// `QUESTION` with its last letter changed: the same length.
const ALTERED: &str = "Synthetic install-rule questioN?";

/// A source rewritten in place — at the same length, or grown with earlier
/// bytes changed — is not the generation that was read: what was read of it
/// is never reused, it is read again. The caller's cached launch, rewritten
/// to print another answer, records nothing for that launch; a neighbour's
/// cached opening, rewritten to the prompt, is a second possible child. The
/// launches whose sources still say what they said are decided as before.
#[test]
fn a_rewritten_source_is_read_again_never_reused() {
    for grown in [false, true] {
        // The caller: read with its result while no child exists yet, then
        // rewritten so its first launch printed another answer.
        let home = Home::new();
        let caller = home.project("-w-caller").join(format!("{CALLER}.jsonl"));
        write_rows(
            &caller,
            &caller_rows(
                CALLER,
                CALL,
                12_000,
                &printed("Answer alpha", "Answer beta"),
            ),
        );
        let mut store = home.store();
        let mut spawns = SpawnBacklog::starting();
        scan(&home, &mut store, &mut spawns);
        let first = settle(&home, &mut store, &mut spawns.bash);
        assert_eq!((first.callers_read, first.waiting), (1, 2), "{first:?}");
        let before = fs::metadata(&caller).unwrap();
        let mut rows = caller_rows(
            CALLER,
            CALL,
            12_000,
            &printed("Answer alphZ", "Answer beta"),
        );
        if grown {
            rows.push(
                json!({"type": "system", "uuid": "late", "timestamp": iso(13_000),
                "sessionId": CALLER, "isSidechain": false}),
            );
        }
        write_rows(&caller, &rows);
        let after = fs::metadata(&caller).unwrap();
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
            assert_eq!(after.len() > before.len(), grown);
            assert!(after.len() >= before.len());
        }
        write_rows(
            &home.project("-w-alpha").join(format!("{ALPHA}.jsonl")),
            &child_rows(ALPHA, "/w/alpha", 3_000, QUESTION, "Answer alpha"),
        );
        write_rows(
            &home.project("-w-beta").join(format!("{BETA}.jsonl")),
            &child_rows(BETA, "/w/beta", 8_000, QUESTION, "Answer beta"),
        );
        scan(&home, &mut store, &mut spawns);
        let total = settle(&home, &mut store, &mut spawns.bash);
        assert!(facts(&store, ALPHA).is_empty(), "caller {grown}: {total:?}");
        assert_eq!(
            known(&home, &store, ALPHA),
            (false, false),
            "caller {grown}"
        );
        assert_eq!(
            facts(&store, BETA),
            [bash(CALLER, CALL, 1)],
            "caller {grown}"
        );

        // A neighbour in the child's folder: its opening read while the
        // child is not there yet, then rewritten to the launch's prompt.
        let home = Home::new();
        standard(&home);
        let alpha = home.project("-w-alpha").join(format!("{ALPHA}.jsonl"));
        let other = home.project("-w-alpha").join(format!("{OTHER}.jsonl"));
        fs::remove_file(&alpha).unwrap();
        write_rows(
            &other,
            &child_rows(OTHER, "/w/alpha", 4_000, ALTERED, "Answer alpha"),
        );
        let mut store = home.store();
        let mut spawns = SpawnBacklog::starting();
        scan(&home, &mut store, &mut spawns);
        let first = settle(&home, &mut store, &mut spawns.bash);
        assert_eq!(facts(&store, BETA), [bash(CALLER, CALL, 1)], "{first:?}");
        let mut rows = child_rows(OTHER, "/w/alpha", 4_000, QUESTION, "Answer alpha");
        if grown {
            rows.push(
                json!({"type": "system", "uuid": "late", "timestamp": iso(9_000),
                "sessionId": OTHER}),
            );
        }
        write_rows(&other, &rows);
        write_rows(
            &alpha,
            &child_rows(ALPHA, "/w/alpha", 3_000, QUESTION, "Answer alpha"),
        );
        scan(&home, &mut store, &mut spawns);
        let total = settle(&home, &mut store, &mut spawns.bash);
        assert!(
            facts(&store, ALPHA).is_empty(),
            "neighbour {grown}: {total:?}"
        );
        assert_eq!(
            known(&home, &store, ALPHA),
            (false, false),
            "neighbour {grown}"
        );
        assert_eq!(
            facts(&store, BETA),
            [bash(CALLER, CALL, 1)],
            "neighbour {grown}"
        );
    }
}

/// Within one run, with one backlog: a second possible child of a launch
/// imported after the launch's fact was recorded withholds that fact, the
/// unchanged caller reused rather than read again. The other launch's
/// independent fact stays accepted.
#[test]
fn a_later_second_possible_child_withholds_within_the_same_run() {
    for max_pass_entries in [LaunchLimits::default().max_pass_entries, 1] {
        same_run_second_child(max_pass_entries);
    }
}

fn same_run_second_child(max_pass_entries: u64) {
    let home = Home::new();
    standard(&home);
    let mut store = home.store();
    let mut spawns = SpawnBacklog::starting();
    scan(&home, &mut store, &mut spawns);
    let limits = LaunchLimits {
        max_pass_entries,
        ..LaunchLimits::default()
    };
    let first = settle_with(&home, &mut store, &mut spawns.bash, &limits, 20_000);
    assert_eq!((first.children, first.changed), (2, 2), "{first:?}");
    assert_eq!(known(&home, &store, ALPHA), (true, false));
    write_rows(
        &home.project("-w-alpha").join(format!("{OTHER}.jsonl")),
        &child_rows(OTHER, "/w/alpha", 4_000, QUESTION, "Answer alpha"),
    );
    scan(&home, &mut store, &mut spawns);
    let after = settle_with(&home, &mut store, &mut spawns.bash, &limits, 20_000);
    assert_eq!(
        after.callers_read, 0,
        "the unchanged caller reused: {after:?}"
    );
    assert_eq!((after.ambiguous, after.changed), (1, 1), "{after:?}");
    let mut withheld = bash(CALLER, CALL, 0);
    withheld.4 = false;
    assert_eq!(facts(&store, ALPHA), [withheld]);
    assert_eq!(known(&home, &store, ALPHA), (false, false));
    assert_eq!(facts(&store, BETA), [bash(CALLER, CALL, 1)]);
    assert_eq!(known(&home, &store, BETA), (true, false));
}

/// A pass whose bytes or directory entries end within a launch's check keeps
/// where the check is — the folder walk, the opening being read, the child
/// read, the walk over the projects — and the next pass goes on from there:
/// a far smaller budget than any one file or folder still reaches the same
/// result, rather than starting over each pass.
#[test]
fn small_budgets_still_reach_a_result() {
    for (name, limits) in [
        (
            "bytes",
            LaunchLimits {
                max_pass_bytes: 97,
                ..LaunchLimits::default()
            },
        ),
        (
            "entries",
            LaunchLimits {
                max_pass_entries: 1,
                ..LaunchLimits::default()
            },
        ),
    ] {
        let home = Home::new();
        standard(&home);
        // Neighbours in the folders: more entries and openings to walk.
        for (folder, cwd, native) in [
            ("-w-alpha", "/w/alpha", OTHER),
            ("-w-beta", "/w/beta", NEIGHBOUR),
        ] {
            write_rows(
                &home.project(folder).join(format!("{native}.jsonl")),
                &child_rows(native, cwd, 5_000, "Something else", "Unrelated"),
            );
        }
        let alpha_len = fs::metadata(home.project("-w-alpha").join(format!("{ALPHA}.jsonl")))
            .unwrap()
            .len();
        assert!(
            name != "bytes" || alpha_len > 3 * limits.max_pass_bytes,
            "{name}"
        );
        let mut store = home.store();
        scan(&home, &mut store, &mut SpawnBacklog::default());
        let total = settle_with(
            &home,
            &mut store,
            &mut BashBacklog::starting(),
            &limits,
            20_000,
        );
        assert_eq!((total.children, total.changed), (2, 2), "{name}: {total:?}");
        assert_eq!(facts(&store, ALPHA), [bash(CALLER, CALL, 0)], "{name}");
        assert_eq!(facts(&store, BETA), [bash(CALLER, CALL, 1)], "{name}");
    }
}
