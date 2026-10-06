//! The reader's automated-input evidence on the import path: Codex and Cursor
//! inputs the tool (or the reader) wrote itself carry a claim on their own
//! line, and a validated claim becomes a proof bound to the stored record —
//! one the import inserts, or one an earlier import already stored. The
//! committed records are the ordinary import's in every case.

use serde_json::{Value, json};
use xt_ingest::native::{
    HostReport, HostStatus, import_reader_lines, import_reader_lines_origin,
    import_reader_lines_tool_sent,
    readers_cli::{ReaderError, ReaderOutcome},
};
use xt_store::{Host, Store, StoredRecord};

const CODEX_NATIVE: &str = "019a0000-0000-7000-8000-000000000001";
const CURSOR_NATIVE: &str = "2a2a2a2a-1111-4222-8333-444444444444";
const CONTRACT: &str = "xtrace.automated_input";

fn uuid(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

fn conversation(host: Host, native: &str) -> String {
    format!("{}-{native}", host.as_str())
}

fn header(host: Host, native: &str) -> String {
    json!({"type":"session","host":host.as_str(),"native_session_id":native,
        "conversation_id":conversation(host, native),"source_surface":null,
        "started_at":"2026-09-30T10:00:00Z","cwd":"/synthetic","git_branch":null,"title":null,
        "path":format!("/synthetic/{}/{native}", host.as_str()),"mtime":1.0})
    .to_string()
}

fn stamp(n: u64) -> String {
    format!("2026-09-30T10:{:02}:{:02}.000Z", n / 60, n % 60)
}

fn user(n: u64, text: &str) -> Value {
    json!({"type":"user","uuid":uuid(n),"timestamp":stamp(n),
        "message":{"role":"user","content":text}})
}

fn assistant(n: u64) -> Value {
    json!({"type":"assistant","uuid":uuid(n),"timestamp":stamp(n),
        "message":{"role":"assistant","content":[{"type":"text","text":"done"}]}})
}

fn claim(kind: &str, native: &str, n: u64) -> Value {
    json!({"contract":CONTRACT,"version":1,"kind":kind,"native_session_id":native,
        "record_uuid":uuid(n)})
}

fn with(mut record: Value, claim: Value) -> Value {
    record["automated_input"] = claim;
    record
}

fn session(host: Host, native: &str, records: &[Value]) -> Vec<String> {
    std::iter::once(header(host, native))
        .chain(records.iter().map(Value::to_string))
        .collect()
}

/// The lines the producer writes without the flag.
fn without_evidence(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            let mut value: Value = serde_json::from_str(line).unwrap();
            value.as_object_mut().unwrap().remove("automated_input");
            value.to_string()
        })
        .collect()
}

fn clean() -> Result<ReaderOutcome, ReaderError> {
    Ok(ReaderOutcome {
        diagnostics: Vec::new(),
        complete: true,
    })
}

fn ok(lines: &[String]) -> Vec<std::io::Result<String>> {
    lines.iter().cloned().map(Ok).collect()
}

fn plain_into(store: &mut Store, host: Host, lines: &[String]) -> HostReport {
    import_reader_lines(store, host, "test".into(), ok(lines), 7, clean)
}

/// The import a scan runs: Codex with its origin evidence (which carries the
/// automated-input claims too), Cursor with the automated-input evidence.
fn flagged_into(store: &mut Store, host: Host, lines: &[String]) -> HostReport {
    match host {
        Host::Codex => import_reader_lines_origin(store, "test".into(), ok(lines), 7, clean),
        _ => import_reader_lines_tool_sent(store, host, "test".into(), ok(lines), 7, clean),
    }
}

fn proofs(db: &Db) -> Vec<(String, String, String)> {
    let connection = rusqlite::Connection::open(&db.path).unwrap();
    let mut statement = connection
        .prepare("SELECT record_uuid,session_id,evidence_kind FROM tool_sent_inputs ORDER BY 1")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// Every stored record with the fields a proof sets cleared, so the ordinary
/// facts compare exactly.
fn ordinary(store: &Store, session: &str) -> Vec<StoredRecord> {
    store
        .records(session)
        .unwrap()
        .into_iter()
        .map(|record| StoredRecord {
            confirmed_automated_input: false,
            human_is_eligible: None,
            human_text_len: None,
            human_excluded: false,
            ..record
        })
        .collect()
}

/// The uuids of the session's user records the index counts as a person's.
fn people(store: &Store, session: &str) -> Vec<String> {
    store
        .records(session)
        .unwrap()
        .into_iter()
        .filter(|record| record.human_is_eligible == Some(true))
        .map(|record| record.uuid)
        .collect()
}

struct Db {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    store: Store,
}

fn db() -> Db {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("index.sqlite");
    let store = Store::open(&path).unwrap();
    Db {
        _dir: dir,
        path,
        store,
    }
}

fn codex_lines() -> Vec<String> {
    session(
        Host::Codex,
        CODEX_NATIVE,
        &[
            user(1, "fix the running indicator"),
            assistant(2),
            with(
                user(3, "<subagent_notification>{}</subagent_notification>"),
                claim("codex_subagent_notification", CODEX_NATIVE, 3),
            ),
            with(
                user(4, "<turn_aborted>\ninterrupted\n</turn_aborted>"),
                claim("codex_turn_aborted", CODEX_NATIVE, 4),
            ),
            with(
                user(
                    5,
                    "<external_codex_apps_open_page>{}</external_codex_apps_open_page>",
                ),
                claim("codex_apps_open_page", CODEX_NATIVE, 5),
            ),
            user(6, "<subagent_notification>pasted</subagent_notification>"),
        ],
    )
}

fn cursor_lines() -> Vec<String> {
    session(
        Host::Cursor,
        CURSOR_NATIVE,
        &[
            with(
                user(1, "[Imported from Cursor · session 2a2a2a2a]"),
                claim("cursor_import_banner", CURSOR_NATIVE, 1),
            ),
            user(2, "rename the module"),
            assistant(3),
            with(
                user(4, "[Previous conversation summary]: renamed"),
                claim("cursor_conversation_summary", CURSOR_NATIVE, 4),
            ),
        ],
    )
}

#[test]
fn a_new_input_is_stored_with_its_proof_and_the_records_are_the_ordinary_ones() {
    for (host, native, lines, kinds) in [
        (
            Host::Codex,
            CODEX_NATIVE,
            codex_lines(),
            vec![
                (3, "codex_subagent_notification"),
                (4, "codex_turn_aborted"),
                (5, "codex_apps_open_page"),
            ],
        ),
        (
            Host::Cursor,
            CURSOR_NATIVE,
            cursor_lines(),
            vec![
                (1, "cursor_import_banner"),
                (4, "cursor_conversation_summary"),
            ],
        ),
    ] {
        let session = conversation(host, native);
        let mut plain = db();
        let plain_report = plain_into(&mut plain.store, host, &without_evidence(&lines));
        let mut flagged = db();
        let flagged_report = flagged_into(&mut flagged.store, host, &lines);
        assert_eq!(
            flagged_report.status,
            HostStatus::Complete,
            "{flagged_report:?}"
        );
        assert_eq!(flagged_report.sessions, plain_report.sessions);
        assert_eq!(
            ordinary(&flagged.store, &session),
            ordinary(&plain.store, &session)
        );
        assert_eq!(
            flagged.store.counts().unwrap(),
            plain.store.counts().unwrap()
        );
        assert_eq!(proofs(&plain), []);
        assert_eq!(
            proofs(&flagged),
            kinds
                .iter()
                .map(|(n, kind)| (uuid(*n), session.clone(), (*kind).to_owned()))
                .collect::<Vec<_>>()
        );
        let claimed = kinds.iter().map(|(n, _)| uuid(*n)).collect::<Vec<_>>();
        let people_before = people(&plain.store, &session);
        let people_after = people(&flagged.store, &session);
        assert_eq!(
            people_after,
            people_before
                .iter()
                .filter(|uuid| !claimed.contains(uuid))
                .cloned()
                .collect::<Vec<_>>()
        );
        assert!(!people_after.is_empty());
    }
}

#[test]
fn an_already_stored_input_is_corrected_when_it_is_read_again() {
    for (host, native, lines, claimed) in [
        (Host::Codex, CODEX_NATIVE, codex_lines(), 3usize),
        (Host::Cursor, CURSOR_NATIVE, cursor_lines(), 2),
    ] {
        let session = conversation(host, native);
        let mut index = db();
        // What a build before this change stored: no claims, every tool-sent
        // input counted as the person's.
        plain_into(&mut index.store, host, &without_evidence(&lines));
        let before = ordinary(&index.store, &session);
        let people_before = people(&index.store, &session);
        assert_eq!(proofs(&index), []);
        // The next scan reads the same session again, with the claims.
        for _ in 0..2 {
            let report = flagged_into(&mut index.store, host, &lines);
            assert_eq!(report.status, HostStatus::Complete, "{report:?}");
            assert_eq!(proofs(&index).len(), claimed);
            assert_eq!(ordinary(&index.store, &session), before);
            assert_eq!(
                people(&index.store, &session).len(),
                people_before.len() - claimed
            );
        }
    }
}

#[test]
fn a_refused_claim_leaves_its_record_the_persons() {
    let good = claim("codex_turn_aborted", CODEX_NATIVE, 1);
    let mut cases: Vec<(&str, Value)> = vec![
        ("contract", json!("memhub.codex.origin_evidence")),
        ("version", json!(2)),
        ("version", json!("1")),
        ("kind", json!("cursor_import_banner")),
        ("kind", json!("claude_task_notification")),
        (
            "native_session_id",
            json!("019a0000-0000-7000-8000-000000000002"),
        ),
        ("record_uuid", json!(uuid(2))),
        ("extra", json!(true)),
    ];
    cases.push(("null", Value::Null));
    for (field, value) in cases {
        let bad = if field == "null" {
            Value::Null
        } else {
            let mut bad = good.clone();
            bad[field] = value;
            bad
        };
        let lines = session(
            Host::Codex,
            CODEX_NATIVE,
            &[
                with(user(1, "<turn_aborted>x</turn_aborted>"), bad),
                user(2, "the person's own"),
            ],
        );
        let mut index = db();
        let report = flagged_into(&mut index.store, Host::Codex, &lines);
        assert_eq!(report.status, HostStatus::Complete, "{field}: {report:?}");
        assert_eq!(proofs(&index), [], "{field}");
        assert_eq!(
            people(&index.store, &conversation(Host::Codex, CODEX_NATIVE)),
            [uuid(1), uuid(2)],
            "{field}"
        );
    }
    // A claim on an assistant line, and a Codex kind on a Cursor session.
    let mut index = db();
    flagged_into(
        &mut index.store,
        Host::Codex,
        &session(
            Host::Codex,
            CODEX_NATIVE,
            &[with(
                assistant(1),
                claim("codex_turn_aborted", CODEX_NATIVE, 1),
            )],
        ),
    );
    flagged_into(
        &mut index.store,
        Host::Cursor,
        &session(
            Host::Cursor,
            CURSOR_NATIVE,
            &[with(
                user(9, "banner"),
                claim("codex_subagent_notification", CURSOR_NATIVE, 9),
            )],
        ),
    );
    assert_eq!(proofs(&index), []);
    // An ordinary import ignores the claims entirely.
    let mut ordinary_import = db();
    plain_into(&mut ordinary_import.store, Host::Codex, &codex_lines());
    assert_eq!(proofs(&ordinary_import), []);
}

/// The pinned producer itself, from the vendored bundle, over a synthetic
/// home: one flat Codex session with a person's ask, a subagent notification,
/// an interrupted-turn note, an app page record and a pasted look-alike, and
/// one Cursor store with a person's ask and Cursor's summary. Read with and
/// without `--automated-input-evidence`, the lines differ only by the
/// evidence. A scan into an index an earlier build filled corrects it: each
/// tool-sent input gets its proof, nothing else changes.
#[test]
fn conformance_tool_sent_import() {
    use sha2::{Digest, Sha256};
    use std::{fs, path::Path};
    use xt_ingest::native::{
        ImportRequest, ProducerSource, import_native,
        readers_cli::{
            read_pin, resolve_python, spawn_codex_reader_with_origin_evidence,
            spawn_cursor_reader_with_tool_sent_evidence, spawn_reader,
        },
    };
    let named = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    let Ok(python) = resolve_python(Some(named.as_os_str()), None) else {
        println!("SKIP conformance_tool_sent_import: Python 3.10+ is unavailable");
        return;
    };
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = ProducerSource::Bundle {
        pin: read_pin(&repo.join(".plugin-pin")).unwrap(),
        root: repo.join("vendor/agent-plugins"),
    };
    let producer = source
        .producer()
        .expect("the bundle verifies against the pin");
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().canonicalize().unwrap().join("home");

    // Codex.
    let sid = "0a0a0a0a-1111-4222-8333-444444444444";
    let stamp = "2026-09-30T10:00:00Z";
    let turn = "0e000000-0000-4000-8000-000000000001";
    let entry =
        |kind: &str, payload: Value| json!({"timestamp":stamp,"type":kind,"payload":payload});
    let input = |n: u64, kind: &str, text: &str| {
        entry(
            "response_item",
            json!({"type":"message","role":"user","id":format!("msg_{n:050x}"),
                "content":[{"type":"input_text","text":text}],
                "internal_chat_message_metadata_passthrough":
                    {"turn_id":turn,"create_time":1.0,"content_item_kinds":[kind]}}),
        )
    };
    let rows = [
        entry(
            "session_meta",
            json!({"id":sid,"timestamp":stamp,"cwd":"/synthetic/project",
                "originator":"codex_cli","history_mode":"legacy"}),
        ),
        entry(
            "event_msg",
            json!({"type":"task_started","turn_id":turn,"started_at":1,
                "collaboration_mode_kind":"default","model_context_window":1}),
        ),
        input(1, "user.text", "fix the running indicator"),
        entry(
            "response_item",
            json!({"type":"message","role":"assistant","id":format!("msg_{:050x}", 2),
                "content":[{"type":"output_text","text":"working"}]}),
        ),
        input(
            3,
            "multi_agent.subagent_notification",
            "<subagent_notification>{}</subagent_notification>",
        ),
        input(
            4,
            "generic.turn_aborted",
            "<turn_aborted>\ninterrupted\n</turn_aborted>",
        ),
        entry(
            "event_msg",
            json!({"type":"turn_aborted","turn_id":turn,"reason":"interrupted"}),
        ),
        input(
            5,
            "additional_content.codex_apps_open_page",
            "<external_codex_apps_open_page>{\"page_id\":null}</external_codex_apps_open_page>",
        ),
        input(
            6,
            "user.text",
            "<subagent_notification>pasted by the person</subagent_notification>",
        ),
    ];
    let dir = home.join(".codex/sessions/2026/09/30");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("rollout-2026-09-30T10-00-00-{sid}.jsonl")),
        rows.iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>(),
    )
    .unwrap();

    // Cursor: a store whose root lists every message leaf in order.
    let cursor_sid = "2a2a2a2a-1111-4222-8333-444444444444";
    let store_dir = home.join(".cursor/chats/19ee0000deadbeef").join(cursor_sid);
    fs::create_dir_all(&store_dir).unwrap();
    fs::write(
        store_dir.join("meta.json"),
        json!({"schemaVersion":1,"cwd":"/synthetic/project","createdAtMs":1790762400000u64,
            "updatedAtMs":1790762460000u64,"hasConversation":true})
        .to_string(),
    )
    .unwrap();
    let messages = [
        json!({"role":"system","content":"harness"}),
        json!({"role":"user","content":[{"type":"text","text":"<user_query>\nrename the module\n</user_query>"}],
            "providerOptions":{"cursor":{"requestId":"r1"}}}),
        json!({"role":"assistant","content":[{"type":"text","text":"Renamed."}],
            "providerOptions":{"cursor":{"modelName":"cursor-test-model"}}}),
        json!({"role":"user","content":"[Previous conversation summary]: the module was renamed",
            "providerOptions":{"cursor":{"isSummary":true}}}),
    ];
    let hex = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let leaves = messages
        .iter()
        .map(|message| {
            let data = message.to_string().into_bytes();
            (hex(&data), data)
        })
        .collect::<Vec<_>>();
    let mut root = Vec::new();
    for (id, _) in &leaves {
        root.extend([0x0a, 0x20]);
        root.extend((0..32).map(|i| u8::from_str_radix(&id[2 * i..2 * i + 2], 16).unwrap()));
    }
    let root_id = hex(&root);
    {
        let connection = rusqlite::Connection::open(store_dir.join("store.db")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);
                 CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);",
            )
            .unwrap();
        for (id, data) in leaves
            .iter()
            .chain(std::iter::once(&(root_id.clone(), root)))
        {
            connection
                .execute(
                    "INSERT INTO blobs VALUES (?1,?2)",
                    rusqlite::params![id, data],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO meta VALUES ('0',?1)",
                [json!({"agentId":cursor_sid,"latestRootBlobId":root_id}).to_string()],
            )
            .unwrap();
    }

    let read = |host: Host, flagged: bool| {
        let (stdout, handle) = match (host, flagged) {
            (Host::Codex, true) => {
                spawn_codex_reader_with_origin_evidence(&python, &producer, &home, None)
            }
            (_, true) => {
                spawn_cursor_reader_with_tool_sent_evidence(&python, &producer, &home, None)
            }
            (_, false) => spawn_reader(&python, &producer, host, &home, None),
        }
        .unwrap();
        let lines = std::io::BufRead::lines(stdout)
            .collect::<std::io::Result<Vec<_>>>()
            .unwrap();
        let outcome = handle.finish().unwrap();
        assert!(
            outcome.complete && outcome.diagnostics.is_empty(),
            "{outcome:?}"
        );
        lines
    };
    let mut index = db();
    for (host, expected) in [
        (
            Host::Codex,
            vec![
                "codex_apps_open_page",
                "codex_subagent_notification",
                "codex_turn_aborted",
            ],
        ),
        (
            Host::Cursor,
            vec!["cursor_conversation_summary", "cursor_import_banner"],
        ),
    ] {
        let plain = read(host, false);
        let flagged = read(host, true);
        let mut kinds = Vec::new();
        let stripped = flagged
            .iter()
            .map(|line| {
                let mut value: Value = serde_json::from_str(line).unwrap();
                let object = value.as_object_mut().unwrap();
                object.remove("origin_evidence");
                if let Some(claim) = object.remove("automated_input") {
                    assert_eq!(claim["record_uuid"], object["uuid"]);
                    kinds.push(claim["kind"].as_str().unwrap().to_owned());
                }
                value
            })
            .collect::<Vec<_>>();
        let plain_values = plain
            .iter()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(stripped, plain_values, "{host:?}");
        kinds.sort();
        assert_eq!(kinds, expected, "{host:?}");
        // What a build before this change stored.
        let report = plain_into(&mut index.store, host, &plain);
        assert_eq!(report.status, HostStatus::Complete, "{report:?}");
    }
    let sessions = [
        conversation(Host::Codex, sid),
        conversation(Host::Cursor, cursor_sid),
    ];
    let before = sessions
        .iter()
        .map(|session| ordinary(&index.store, session))
        .collect::<Vec<_>>();
    let people_before = sessions
        .iter()
        .map(|session| people(&index.store, session).len())
        .collect::<Vec<_>>();
    assert_eq!(proofs(&index), []);

    // The scan itself reads both hosts with their evidence and corrects them.
    let report = import_native(
        &mut index.store,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Codex, Host::Cursor],
            producer: &source,
            python: Some(python.as_os_str()),
            observed_at: 7,
            cancel: None,
        },
    );
    for host in &report.hosts {
        assert_eq!(host.status, HostStatus::Complete, "{host:?}");
    }
    let mut kinds = proofs(&index)
        .into_iter()
        .map(|(_, session, kind)| (session, kind))
        .collect::<Vec<_>>();
    kinds.sort();
    assert_eq!(
        kinds,
        [
            (sessions[0].clone(), "codex_apps_open_page".to_owned()),
            (
                sessions[0].clone(),
                "codex_subagent_notification".to_owned()
            ),
            (sessions[0].clone(), "codex_turn_aborted".to_owned()),
            (
                sessions[1].clone(),
                "cursor_conversation_summary".to_owned()
            ),
            (sessions[1].clone(), "cursor_import_banner".to_owned()),
        ]
    );
    for (n, session) in sessions.iter().enumerate() {
        assert_eq!(ordinary(&index.store, session), before[n], "{session}");
    }
    // Codex: the ask and the pasted look-alike stay the person's. Cursor: the
    // ask does.
    assert_eq!(people(&index.store, &sessions[0]).len(), 2);
    assert_eq!(people_before[0], 5);
    assert_eq!(people(&index.store, &sessions[1]).len(), 1);
    assert_eq!(people_before[1], 3);
}
