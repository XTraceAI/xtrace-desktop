//! Native Claude `pr-link` witnesses: an explicit line naming a pull request
//! becomes one content-free stub and one exact link for the session it names,
//! committed in the same transaction as the rows and checkpoint covering it.
//! Every fixture is synthetic; nothing here touches Git, GitHub or a network.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use xt_ingest::{
    canonical::{Parsed, SourceContext, parse_line},
    native::{
        ImportReport, ImportRequest, ProducerSource, ScanMode, SessionOutcome,
        scan_native_observed,
        stream::{StreamEvent, StreamEvents},
    },
    writer::{PrWitness, WriteBatch, write_batch},
};
use xt_store::{
    Host, SessionSource, Store,
    ingest::DiscoveredSession,
    pr_link::{PrConfidence, PrIdentity},
};

const SID: &str = "00000000-0000-4000-8000-000000000001";
const FORK: &str = "00000000-0000-4000-8000-0000000000f2";
const URL: &str = "https://github.com/example/fixture/pull/42";
const SEEN: &str = "2026-09-07T12:00:02Z";
/// `SEEN` in UTC milliseconds.
const SEEN_MS: i64 = 1_788_782_402_000;
/// Scan clocks are far from every witness time, so a link can never borrow one.
const SCAN_AT: i64 = 1_900_000_000_000;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn record(uuid: &str, session: &str) -> String {
    json!({"uuid":uuid,"type":"user","sessionId":session,"entrypoint":"cli",
        "timestamp":"2026-09-07T12:00:00Z","cwd":"/repo/fixture","gitBranch":"fixture-branch",
        "message":{"role":"user","content":[{"type":"text","text":"Synthetic private prompt."}]}})
    .to_string()
}

fn witness(session: &str, number: u64, timestamp: &str) -> Value {
    json!({"type":"pr-link","sessionId":session,"prNumber":number,
        "prUrl":format!("https://github.com/example/fixture/pull/{number}"),
        "prRepository":"example/fixture","timestamp":timestamp})
}

fn lines(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

fn project(home: &Path) -> PathBuf {
    let project = home.join(".claude/projects/-Users-fixture-repo");
    fs::create_dir_all(&project).unwrap();
    project
}

fn set_mtime(path: &Path, seconds: u64) {
    fs::File::open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
        ))
        .unwrap();
}

fn hashes(root: &Path) -> BTreeMap<PathBuf, String> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.insert(
                    path.clone(),
                    format!("{:x}", Sha256::digest(fs::read(&path).unwrap())),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut out);
    out
}

fn scan(store: &mut Store, home: &Path, mode: ScanMode) -> ImportReport {
    static CLOCK: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(SCAN_AT);
    scan_native_observed(
        store,
        &ImportRequest {
            home,
            hosts: &[Host::Claude],
            producer: &ProducerSource::Checkout {
                pin: repo().join(".plugin-pin"),
                plugin_root: None,
            },
            python: None,
            observed_at: CLOCK.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            cancel: None,
        },
        mode,
        &mut |_| {},
    )
}

fn run(store: &mut Store, home: &Path) -> ImportReport {
    scan(store, home, ScanMode::Resume)
}

fn count(path: &Path, query: &str) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row(query, [], |row| row.get(0))
        .unwrap()
}

fn links(store: &Store) -> Vec<(String, String, PrConfidence, i64, i64)> {
    store
        .all_pr_links()
        .unwrap()
        .into_iter()
        .map(|link| {
            (
                link.session_id,
                link.pull_request.url(),
                link.confidence,
                link.first_seen_at,
                link.last_seen_at,
            )
        })
        .collect()
}

#[test]
fn an_exact_witness_persists_a_content_free_stub_and_link_at_its_own_time() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    // The witness spells the repository differently from its URL, which
    // reconciles; the canonical row is lowercase.
    let mut line = witness(SID, 42, SEEN);
    line["prRepository"] = json!("Example/Fixture");
    fs::write(
        project(&home).join(format!("{SID}.jsonl")),
        lines(&[
            record("00000000-0000-4000-8000-0000000000a1", SID),
            line.to_string(),
        ]),
    )
    .unwrap();
    let before = hashes(&home);
    let path = temp.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    let report = run(&mut store, &home);
    assert!(report.complete(), "{report:?}");

    assert_eq!(
        links(&store),
        [(
            SID.to_owned(),
            URL.to_owned(),
            PrConfidence::Exact,
            SEEN_MS,
            SEEN_MS
        )]
    );
    let identity = PrIdentity::from_url(URL).unwrap();
    let stub = store.pull_request(&identity).unwrap().unwrap();
    assert_eq!(stub.identity, identity);
    assert_eq!(
        (
            stub.state,
            stub.merged_at,
            stub.additions,
            stub.deletions,
            stub.refreshed_at
        ),
        (None, None, None, None, None),
        "a link never implies merge or refresh state"
    );
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM pull_requests WHERE title IS NOT NULL OR head_ref_name IS NOT NULL"
        ),
        0
    );
    // The witness is no record, no tool event and no content.
    assert_eq!(count(&path, "SELECT count(*) FROM records"), 1);
    assert_eq!(count(&path, "SELECT count(*) FROM tool_uses"), 0);
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM records WHERE content_json IS NOT NULL"
        ),
        0,
        "the default policy keeps metadata only"
    );
    assert_eq!(count(&path, "SELECT count(*) FROM native_checkpoints"), 1);
    assert_eq!(hashes(&home), before, "native sources are only ever read");
}

#[test]
fn duplicate_reversed_and_replayed_witnesses_converge_on_one_link() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    // The same pull request witnessed three times, the later time first.
    fs::write(
        project(&home).join(format!("{SID}.jsonl")),
        lines(&[
            record("00000000-0000-4000-8000-0000000000a1", SID),
            witness(SID, 42, "2026-09-07T12:05:00Z").to_string(),
            witness(SID, 42, SEEN).to_string(),
            witness(SID, 42, SEEN).to_string(),
        ]),
    )
    .unwrap();
    let path = temp.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    let expected = [(
        SID.to_owned(),
        URL.to_owned(),
        PrConfidence::Exact,
        SEEN_MS,
        SEEN_MS + 298_000,
    )];
    for mode in [ScanMode::Resume, ScanMode::Replay, ScanMode::Replay] {
        let report = scan(&mut store, &home, mode);
        assert!(report.complete(), "{mode:?}: {report:?}");
        assert_eq!(links(&store), expected, "{mode:?}");
        assert_eq!(count(&path, "SELECT count(*) FROM pull_requests"), 1);
    }
}

#[test]
fn two_pull_requests_are_two_stubs_and_two_links() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    fs::write(
        project(&home).join(format!("{SID}.jsonl")),
        lines(&[
            record("00000000-0000-4000-8000-0000000000a1", SID),
            witness(SID, 42, SEEN).to_string(),
            witness(SID, 7, "2026-09-07T12:10:00Z").to_string(),
        ]),
    )
    .unwrap();
    let path = temp.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    assert!(run(&mut store, &home).complete());
    let stored = store.session_pr_links(SID).unwrap();
    assert_eq!(
        stored
            .iter()
            .map(|link| (link.pull_request.number(), link.first_seen_at))
            .collect::<Vec<_>>(),
        [(7, SEEN_MS + 598_000), (42, SEEN_MS)]
    );
    assert_eq!(count(&path, "SELECT count(*) FROM pull_requests"), 2);
}

/// A fork's inherited prefix still names its original session. The witness
/// there is the original's evidence: it is never the fork's link, both files
/// hold one link between them, and the result is the same in either scan order.
#[test]
fn an_inherited_copy_is_one_canonical_link_in_either_scan_order() {
    for fork_first in [false, true] {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("home");
        let project = project(&home);
        let shared = [
            record("00000000-0000-4000-8000-0000000000a1", SID),
            witness(SID, 42, SEEN).to_string(),
        ];
        let original = project.join(format!("{SID}.jsonl"));
        let fork = project.join(format!("{FORK}.jsonl"));
        fs::write(&original, lines(&shared)).unwrap();
        let mut forked = shared.to_vec();
        forked.push(record("00000000-0000-4000-8000-0000000000a2", FORK));
        fs::write(&fork, lines(&forked)).unwrap();
        set_mtime(&original, if fork_first { 100 } else { 200 });
        set_mtime(&fork, if fork_first { 200 } else { 100 });
        let before = hashes(&home);
        let path = temp.path().join("index.sqlite");
        let mut store = Store::open(&path).unwrap();
        for pass in 0..2 {
            let report = run(&mut store, &home);
            assert!(report.complete(), "fork_first={fork_first} {report:?}");
            assert_eq!(
                links(&store),
                [(
                    SID.to_owned(),
                    URL.to_owned(),
                    PrConfidence::Exact,
                    SEEN_MS,
                    SEEN_MS
                )],
                "fork_first={fork_first} pass={pass}"
            );
            assert!(store.session_pr_links(FORK).unwrap().is_empty());
            assert_eq!(count(&path, "SELECT count(*) FROM pull_requests"), 1);
        }
        assert_eq!(hashes(&home), before);
    }
}

/// A copy naming a session the index does not hold attaches nowhere: not to
/// the named session, which does not exist, and not to the fork in its place.
/// It is not a gap in the fork, so its checkpoint still advances; the named
/// session's own file, once scanned, links it.
#[test]
fn an_inherited_copy_of_an_unindexed_session_attaches_nowhere() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let project = project(&home);
    fs::write(
        project.join(format!("{FORK}.jsonl")),
        lines(&[
            witness(SID, 42, SEEN).to_string(),
            record("00000000-0000-4000-8000-0000000000a2", FORK),
        ]),
    )
    .unwrap();
    let path = temp.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    assert!(run(&mut store, &home).complete());
    assert!(links(&store).is_empty());
    assert_eq!(count(&path, "SELECT count(*) FROM pull_requests"), 0);
    assert_eq!(
        count(
            &path,
            &format!("SELECT count(*) FROM sessions WHERE session_id='{SID}'")
        ),
        0,
        "no session is manufactured for the named one"
    );
    assert_eq!(count(&path, "SELECT count(*) FROM sessions"), 1);
    assert_eq!(count(&path, "SELECT count(*) FROM native_checkpoints"), 1);

    fs::write(
        project.join(format!("{SID}.jsonl")),
        lines(&[witness(SID, 42, SEEN).to_string()]),
    )
    .unwrap();
    assert!(run(&mut store, &home).complete());
    assert_eq!(
        links(&store),
        [(
            SID.to_owned(),
            URL.to_owned(),
            PrConfidence::Exact,
            SEEN_MS,
            SEEN_MS
        )]
    );
}

/// A witness is reconciled like a record, and it needs its session and its own
/// time. Every disagreement or malformed identity stops the file before the
/// batch covering the witness commits: no record, stub, link or checkpoint.
#[test]
fn witness_identity_failures_commit_neither_link_nor_checkpoint() {
    let mut cases = Vec::new();
    let mut add = |name: &str, edit: &dyn Fn(&mut Value)| {
        let mut line = witness(SID, 42, SEEN);
        edit(&mut line);
        cases.push((name.to_owned(), line.to_string()));
    };
    add("missing session", &|line| {
        line.as_object_mut().unwrap().remove("sessionId");
    });
    add("blank session", &|line| line["sessionId"] = json!("  "));
    add("native session spellings disagree", &|line| {
        line["native_session_id"] = json!(FORK)
    });
    add("conversation disagrees", &|line| {
        line["conversation_id"] = json!(FORK)
    });
    add("platform disagrees", &|line| {
        line["source_platform"] = json!("codex")
    });
    add("source disagrees", &|line| line["source"] = json!("plugin"));
    add("surface disagrees", &|line| {
        line["source_surface"] = json!("desktop")
    });
    add("entrypoint disagrees", &|line| {
        line["entrypoint"] = json!("sdk-ts")
    });
    add("missing timestamp", &|line| {
        line.as_object_mut().unwrap().remove("timestamp");
    });
    add("invalid timestamp", &|line| {
        line["timestamp"] = json!("yesterday")
    });
    add("URL host", &|line| {
        line["prUrl"] = json!("https://gitlab.com/example/fixture/pull/42")
    });
    add("URL alias", &|line| {
        line["prUrl"] = json!("https://github.com/example/fixture.git/pull/42")
    });
    add("repository disagrees", &|line| {
        line["prRepository"] = json!("example/other")
    });
    add("number disagrees", &|line| line["prNumber"] = json!(43));
    add("zero number", &|line| line["prNumber"] = json!(0));
    add("blank URL", &|line| line["prUrl"] = json!(" "));
    for (name, line) in cases {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("home");
        fs::write(
            project(&home).join(format!("{SID}.jsonl")),
            lines(&[record("00000000-0000-4000-8000-0000000000a1", SID), line]),
        )
        .unwrap();
        let path = temp.path().join("index.sqlite");
        let mut store = Store::open(&path).unwrap();
        let report = run(&mut store, &home);
        assert!(
            matches!(
                report.hosts[0].sessions[0].outcome,
                SessionOutcome::Skipped { .. }
            ),
            "{name}: {:?}",
            report.hosts[0].sessions[0].outcome
        );
        for table in ["records", "pull_requests", "pr_links", "native_checkpoints"] {
            assert_eq!(
                count(&path, &format!("SELECT count(*) FROM {table}")),
                0,
                "{name}: {table}"
            );
        }
    }
}

/// A legacy pull-request row the link write reports as a conflict fails the
/// whole batch: the records beside the witness and the checkpoint covering it
/// roll back, and the legacy row is left exactly as it was.
#[test]
fn a_link_conflict_rolls_back_the_whole_batch_and_its_checkpoint() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let source = project(&home).join(format!("{SID}.jsonl"));
    fs::write(
        &source,
        lines(&[
            record("00000000-0000-4000-8000-0000000000a1", SID),
            witness(SID, 42, SEEN).to_string(),
        ]),
    )
    .unwrap();
    let path = temp.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute(
            "INSERT INTO pull_requests(repo,number,url) VALUES ('Example/Fixture',42,?1)",
            ["https://github.com/Example/Fixture/pull/42"],
        )
        .unwrap();
    let report = run(&mut store, &home);
    assert!(
        matches!(
            report.hosts[0].sessions[0].outcome,
            SessionOutcome::Skipped { .. }
        ),
        "{report:?}"
    );
    for table in [
        "records",
        "pr_links",
        "native_checkpoints",
        "session_sources",
    ] {
        assert_eq!(
            count(&path, &format!("SELECT count(*) FROM {table}")),
            0,
            "{table}"
        );
    }
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM pull_requests WHERE repo='Example/Fixture' AND url='https://github.com/Example/Fixture/pull/42'"
        ),
        1
    );
}

/// A recorded checkpoint survives a restart: an unchanged file is not read
/// again, so a witness is not replayed; an appended witness-only tail is read
/// behind the checkpoint and carries its own progress.
#[test]
fn a_restart_resumes_behind_the_checkpoint_without_replaying_witnesses() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let source = project(&home).join(format!("{SID}.jsonl"));
    fs::write(
        &source,
        lines(&[
            record("00000000-0000-4000-8000-0000000000a1", SID),
            witness(SID, 42, SEEN).to_string(),
        ]),
    )
    .unwrap();
    let path = temp.path().join("index.sqlite");
    {
        let mut store = Store::open(&path).unwrap();
        assert!(run(&mut store, &home).complete());
    }
    // Prove the next resume reads nothing: a link removed behind the scan's
    // back stays removed while the file is unchanged.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM pr_links", [])
        .unwrap();
    let checkpoint = || -> (i64, i64) {
        rusqlite::Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT count(*), max(position) FROM native_checkpoints",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    };
    let recorded = checkpoint();
    let mut store = Store::open(&path).unwrap();
    let report = run(&mut store, &home);
    assert!(report.complete(), "{report:?}");
    assert!(links(&store).is_empty(), "an unchanged file was replayed");
    assert_eq!(checkpoint(), recorded);

    // A witness-only append is read behind the checkpoint and advances it.
    let mut appended = fs::OpenOptions::new().append(true).open(&source).unwrap();
    std::io::Write::write_all(
        &mut appended,
        lines(&[witness(SID, 7, "2026-09-07T12:10:00Z").to_string()]).as_bytes(),
    )
    .unwrap();
    drop(appended);
    assert!(run(&mut store, &home).complete());
    assert_eq!(
        store
            .session_pr_links(SID)
            .unwrap()
            .iter()
            .map(|link| link.pull_request.number())
            .collect::<Vec<_>>(),
        [7]
    );
    assert!(checkpoint().1 > recorded.1);
    drop(store);

    // A deliberate replay restores what the resume did not reread.
    let mut store = Store::open(&path).unwrap();
    assert!(scan(&mut store, &home, ScanMode::Replay).complete());
    assert_eq!(store.session_pr_links(SID).unwrap().len(), 2);
}

fn claude_context(session: &str, source: SessionSource) -> SourceContext {
    SourceContext {
        conversation_id: Some(session.into()),
        native_session_id: Some(session.into()),
        source_platform: Some("claude".into()),
        source_surface: Some("cli".into()),
        started_at: None,
        source: Some(source),
    }
}

fn parsed_witness(line: &Value) -> PrWitness {
    let Parsed::PrLink(link) = parse_line(&line.to_string()).unwrap() else {
        panic!("expected a PR witness");
    };
    let named_session = link.native.session_id.clone().unwrap();
    PrWitness {
        link: *link,
        named_session,
    }
}

fn batch<'a>(
    context: &'a SourceContext,
    witnesses: &'a [PrWitness],
    discovery: Option<&'a DiscoveredSession>,
) -> WriteBatch<'a> {
    WriteBatch {
        context,
        declared_host: Some(Host::Claude),
        records: &[],
        hook_summaries: &[],
        pr_witnesses: witnesses,
        title: None,
        cwd: None,
        git_branch: None,
        namespace: None,
        keep_content: false,
        observed_at: SCAN_AT,
        receipt: None,
        cursor: None,
        discovery,
        checkpoint: None,
    }
}

/// Only discovered native Claude transcript history carries witnesses: a
/// generic transcript import, a plugin capture or another host writes nothing.
#[test]
fn the_writer_accepts_witnesses_only_from_discovered_claude_history() {
    let db = xt_fixtures::TempDb::empty().unwrap();
    let mut store = Store::open(db.path()).unwrap();
    let witnesses = [parsed_witness(&witness(SID, 42, SEEN))];
    let discovery = DiscoveredSession {
        host: Host::Claude,
        native_session_id: SID.into(),
        conversation_id: Some(SID.into()),
        surface: Some("cli".into()),
        started_at_ms: None,
        last_observed_at: SCAN_AT,
        discovery_complete: true,
    };
    let transcript = claude_context(SID, SessionSource::Transcript);
    assert!(write_batch(&mut store, &batch(&transcript, &witnesses, None)).is_err());
    let readers = claude_context(SID, SessionSource::ReadersCli);
    assert!(write_batch(&mut store, &batch(&readers, &witnesses, Some(&discovery))).is_err());
    assert_eq!(store.counts().unwrap().sessions, 0);
    assert!(links(&store).is_empty());

    // The witness's own labels are checked against the batch, as a record's are.
    let mut foreign = witnesses[0].clone();
    foreign.link.source.source_platform = Some("codex".into());
    assert!(
        write_batch(
            &mut store,
            &batch(&transcript, &[foreign], Some(&discovery))
        )
        .is_err()
    );
    let mut untimed = witnesses[0].clone();
    untimed.link.timestamp = None;
    assert!(
        write_batch(
            &mut store,
            &batch(&transcript, &[untimed], Some(&discovery))
        )
        .is_err()
    );
    assert_eq!(store.counts().unwrap().sessions, 0);

    write_batch(
        &mut store,
        &batch(&transcript, &witnesses, Some(&discovery)),
    )
    .unwrap();
    assert_eq!(
        links(&store),
        [(
            SID.to_owned(),
            URL.to_owned(),
            PrConfidence::Exact,
            SEEN_MS,
            SEEN_MS
        )]
    );
}

/// The shared reader stream protocol is unchanged: a `pr-link` line inside a
/// reader-produced session is still outside the contract and ends it.
#[test]
fn the_shared_reader_stream_still_rejects_pr_link_lines() {
    let mut stream = StreamEvents::new(Host::Codex, SessionSource::ReadersCli);
    let header = json!({"type":"session","host":"codex","native_session_id":"n",
        "conversation_id":"codex-n","path":"/synthetic","mtime":1.0});
    assert!(matches!(
        stream.push(&header.to_string()).unwrap(),
        Some(StreamEvent::Session(_))
    ));
    assert!(matches!(
        stream.push(&witness("n", 42, SEEN).to_string()).unwrap(),
        Some(StreamEvent::MalformedRecord { .. })
    ));
}

/// An index an earlier build wrote holds a checkpoint past witnesses it
/// ignored. Migration 7 forgets transcript checkpoints once, so the next
/// ordinary resume replays the unchanged file and links the witness; records
/// are not duplicated and the recreated checkpoint then holds across restarts.
#[test]
fn a_v6_index_replays_unchanged_transcripts_once_and_links_their_witnesses() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    fs::write(
        project(&home).join(format!("{SID}.jsonl")),
        lines(&[
            record("00000000-0000-4000-8000-0000000000a1", SID),
            witness(SID, 42, SEEN).to_string(),
        ]),
    )
    .unwrap();
    let before = hashes(&home);
    let path = temp.path().join("index.sqlite");
    {
        let mut store = Store::open(&path).unwrap();
        assert!(run(&mut store, &home).complete());
    }
    // What a schema-6 build left behind: the rows and a checkpoint, no link
    // and no refresh status columns.
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch(
        "DELETE FROM pr_links; DELETE FROM pull_requests;
         DELETE FROM schema_version WHERE version>=7;
         ALTER TABLE pull_requests DROP COLUMN refresh_error; ALTER TABLE pull_requests DROP COLUMN last_attempted_at;
         ALTER TABLE tool_uses DROP COLUMN group_key;
         ALTER TABLE tool_uses DROP COLUMN group_version;
         ALTER TABLE tool_uses DROP COLUMN group_conflict;
         DROP TABLE confirmed_automated_inputs;
         DROP TABLE guardian_turn_inputs;
         DROP TABLE injected_context_inputs; DROP TABLE IF EXISTS task_notification_inputs; DROP TABLE IF EXISTS record_previews; DROP TABLE IF EXISTS human_input_adjustments; DROP TABLE IF EXISTS human_session_origins;
         DROP TABLE session_creation_relations;
         DROP TABLE session_creation_bootstrap; DROP TABLE cli_artifact_launch_owners; DROP TABLE claude_launch_groups; DROP TABLE claude_launch_group_members; DROP TABLE claude_launch_candidates; DROP TABLE claude_launch_staged_candidates;
         DROP INDEX sessions_host_native; DROP INDEX source_cursors_tail;",
    )
    .unwrap();
    assert_eq!(count(&path, "SELECT count(*) FROM native_checkpoints"), 1);

    let linked = [(
        SID.to_owned(),
        URL.to_owned(),
        PrConfidence::Exact,
        SEEN_MS,
        SEEN_MS,
    )];
    for pass in 0..2 {
        let mut store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 17);
        let report = run(&mut store, &home);
        assert!(report.complete(), "pass {pass}: {report:?}");
        if pass == 0 {
            assert_eq!(links(&store), linked, "the upgrade replay links");
            // Removed behind the scan's back, so a reread would show.
            sql.execute("DELETE FROM pr_links", []).unwrap();
        } else {
            assert!(links(&store).is_empty(), "the recreated checkpoint held");
        }
        assert_eq!(count(&path, "SELECT count(*) FROM records"), 1);
        assert_eq!(count(&path, "SELECT count(*) FROM native_checkpoints"), 1);
    }
    assert_eq!(hashes(&home), before);
}
