//! The Codex origin-evidence import: synthetic streams only, each imported
//! twice into fresh stores, with its evidence and as the ordinary producer
//! emits it without the flag. The committed records must be the ordinary
//! import's in every case; only a validated, undisputed claim adds a proof,
//! and only in the transaction that inserts its record.

use serde_json::{Value, json};
use std::collections::BTreeMap;
use xt_ingest::native::{
    HostReport, HostStatus, ProvenWrite, SessionOutcome, SessionWriter, import_reader_lines,
    import_reader_lines_origin,
    readers_cli::{ReaderDiagnostic, ReaderError, ReaderOutcome},
    stream::{StreamEvent, StreamEvents},
};
use xt_store::{
    Host, SessionSource, Store, StoredRecord, StoredSession,
    injected::{InjectedContextKind, InjectedContextProof, OriginContract, SegmentHistory},
};

const NATIVE: &str = "019a0000-0000-7000-8000-000000000001";
const OTHER: &str = "019a0000-0000-7000-8000-000000000002";
const TURN: &str = "019a0000-0000-7000-8000-0000000000f1";
const CONTRACT: &str = "memhub.codex.origin_evidence";
const KIND: &str = "codex_selected_skill_instructions";
const SKILL_BODY: &str = "<skill>\n# Review\nFollow these steps.\n</skill>";
const MAX: usize = xt_ingest::writer::MAX_BATCH_RECORDS;

fn uuid(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

fn item(n: u64) -> String {
    format!("msg_00000000-0000-4000-9000-{n:012x}")
}

fn path(native: &str) -> String {
    format!("/synthetic/.codex/sessions/rollout-{native}.jsonl")
}

fn header(native: &str, marked: bool) -> String {
    let mut header = json!({"type":"session","host":"codex","native_session_id":native,
        "conversation_id":format!("codex-{native}"),"source_surface":"codex_cli",
        "started_at":"2026-09-25T10:00:00Z","cwd":"/synthetic","git_branch":null,"title":null,
        "path":path(native),"mtime":1.0});
    if marked {
        header["origin_evidence"] = json!({"contract":CONTRACT,"version":1});
    }
    header.to_string()
}

fn stamp(n: u64) -> String {
    format!(
        "2026-09-25T{:02}:{:02}:{:02}.{:03}Z",
        10 + n / 3_600_000,
        (n / 60_000) % 60,
        (n / 1_000) % 60,
        n % 1_000
    )
}

fn user(n: u64, text: &str) -> Value {
    json!({"type":"user","uuid":uuid(n),"timestamp":stamp(n),
        "message":{"role":"user","content":text}})
}

fn assistant(n: u64) -> Value {
    json!({"type":"assistant","uuid":uuid(n),"timestamp":stamp(n),
        "message":{"role":"assistant","content":[{"type":"text","text":"done"}]}})
}

fn claim(native: &str, n: u64, row: u64) -> Value {
    json!({"contract":CONTRACT,"version":1,"kind":KIND,"native_session_id":native,
        "segment":{"history":"flat","rollout_id":null},"row":{"index":row,"ordinal":null},
        "item_id":item(n),"turn_id":TURN,"record_uuid":uuid(n)})
}

fn with(mut record: Value, claim: Value) -> Value {
    record["origin_evidence"] = claim;
    record
}

/// The injected body `n`, claimed at native row `n`.
fn skill(n: u64) -> Value {
    with(user(n, SKILL_BODY), claim(NATIVE, n, n))
}

fn proof(native: &str, n: u64, row: u32) -> InjectedContextProof {
    InjectedContextProof {
        contract: OriginContract::CodexOriginEvidence,
        version: 1,
        kind: InjectedContextKind::CodexSelectedSkillInstructions,
        native_session_id: native.into(),
        history: SegmentHistory::Flat,
        rollout_id: None,
        row_index: row,
        row_ordinal: None,
        item_id: item(n),
        turn_id: TURN.into(),
        record_uuid: uuid(n),
    }
}

fn session(native: &str, marked: bool, records: &[Value]) -> Vec<String> {
    std::iter::once(header(native, marked))
        .chain(records.iter().map(Value::to_string))
        .collect()
}

/// `count` ordinary records numbered from `from`, alternating user and
/// assistant, none claimed.
fn filler(from: u64, count: u64) -> Vec<Value> {
    (from..from + count)
        .map(|n| {
            if n % 2 == 0 {
                user(n, &format!("turn {n}"))
            } else {
                assistant(n)
            }
        })
        .collect()
}

/// The stream the producer writes without the flag: the same lines, less
/// every marker and claim.
fn without_evidence(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| match serde_json::from_str::<Value>(line) {
            Ok(Value::Object(mut object)) => {
                object.remove("origin_evidence");
                Value::Object(object).to_string()
            }
            _ => line.clone(),
        })
        .collect()
}

type Finish = fn() -> Result<ReaderOutcome, ReaderError>;

fn clean() -> Result<ReaderOutcome, ReaderError> {
    Ok(ReaderOutcome {
        diagnostics: Vec::new(),
        complete: true,
    })
}

fn failed() -> Result<ReaderOutcome, ReaderError> {
    Err(ReaderError::Failed("reader exited with status 1".into()))
}

fn cancelled() -> Result<ReaderOutcome, ReaderError> {
    Err(ReaderError::Cancelled)
}

fn plain_into(
    store: &mut Store,
    lines: Vec<std::io::Result<String>>,
    finish: Finish,
) -> HostReport {
    import_reader_lines(store, Host::Codex, "test".into(), lines, 7, finish)
}

fn flagged_into(
    store: &mut Store,
    lines: Vec<std::io::Result<String>>,
    finish: Finish,
) -> HostReport {
    import_reader_lines_origin(store, "test".into(), lines, 7, finish)
}

fn ok(lines: &[String]) -> Vec<std::io::Result<String>> {
    lines.iter().cloned().map(Ok).collect()
}

/// Both imports of one stream, each into a fresh store.
fn both(lines: &[String], finish: Finish) -> ((Store, HostReport), (Store, HostReport)) {
    let mut plain = Store::open_in_memory().unwrap();
    let plain_report = plain_into(&mut plain, ok(&without_evidence(lines)), finish);
    let mut flagged = Store::open_in_memory().unwrap();
    let flagged_report = flagged_into(&mut flagged, ok(lines), finish);
    ((plain, plain_report), (flagged, flagged_report))
}

/// Every stored record of the sessions, with the one field a proof sets
/// cleared, so the ordinary facts compare exactly.
fn ordinary(store: &Store, natives: &[&str]) -> Vec<StoredRecord> {
    natives
        .iter()
        .flat_map(|native| store.records(&format!("codex-{native}")).unwrap())
        .map(|record| StoredRecord {
            confirmed_automated_input: false,
            human_is_eligible: None,
            human_text_len: None,
            human_excluded: false,
            ..record
        })
        .collect()
}

fn proofs(store: &Store, native: &str) -> Vec<InjectedContextProof> {
    store
        .injected_context_proofs(&format!("codex-{native}"))
        .unwrap()
}

/// Every stored session row whole: cwd, branch, surface and its evidence,
/// native start, conflict and timestamp range.
fn session_rows(store: &Store, natives: &[&str]) -> Vec<Option<StoredSession>> {
    natives
        .iter()
        .map(|native| store.session(&format!("codex-{native}")).unwrap())
        .collect()
}

/// The flagged import committed exactly the ordinary import's records and
/// session rows and reported the same sessions, and its claims account for
/// themselves.
fn assert_parity(plain: &(Store, HostReport), flagged: &(Store, HostReport), natives: &[&str]) {
    assert_eq!(ordinary(&flagged.0, natives), ordinary(&plain.0, natives));
    assert_eq!(
        session_rows(&flagged.0, natives),
        session_rows(&plain.0, natives)
    );
    assert_eq!(flagged.0.counts().unwrap(), plain.0.counts().unwrap());
    assert_eq!(flagged.1.sessions, plain.1.sessions);
    assert_eq!(flagged.1.status, plain.1.status);
    let claims = &flagged
        .1
        .origin
        .as_ref()
        .expect("origin mode reports")
        .claims;
    let abstained = &claims.abstained;
    assert_eq!(
        claims.claimed,
        claims.recorded
            + abstained.not_inserted
            + abstained.conflicted_record
            + abstained.ineligible
            + abstained.conflicting_proof
            + claims.disputed
            + claims.store_refused
            + claims.over_cap
            + claims.unholdable
            + claims.unwritten,
        "{claims:?}"
    );
}

fn cursor(store: &Store, native: &str) -> bool {
    store
        .source_cursor(
            SessionSource::ReadersCli,
            &format!("codex:{}", path(native)),
        )
        .unwrap()
        .is_some()
}

#[test]
fn a_fresh_skill_claim_is_recorded_with_its_record_and_the_persons_inputs_stay_human() {
    let lines = session(
        NATIVE,
        true,
        &[
            // The person's own `$skill` ask beside the injected body.
            user(1, "$review please check this branch"),
            skill(2),
            // Pasted text identical to the body, quoting a claim, is human.
            user(
                3,
                &format!(
                    "{SKILL_BODY} {}",
                    json!({"origin_evidence": claim(NATIVE, 3, 3)})
                ),
            ),
            assistant(4),
        ],
    );
    let (plain, flagged) = both(&lines, clean);
    assert_parity(&plain, &flagged, &[NATIVE]);
    let report = &flagged.1;
    assert_eq!(report.status, HostStatus::Complete, "{report:?}");
    let origin = report.origin.as_ref().unwrap();
    assert_eq!(origin.sessions.marked, 1);
    assert_eq!((origin.claims.claimed, origin.claims.recorded), (1, 1));
    assert_eq!(proofs(&flagged.0, NATIVE), [proof(NATIVE, 2, 2)]);
    assert!(proofs(&plain.0, NATIVE).is_empty());
    for record in flagged.0.records(&format!("codex-{NATIVE}")).unwrap() {
        assert_eq!(record.confirmed_automated_input, record.uuid == uuid(2));
        if record.uuid == uuid(1) || record.uuid == uuid(3) {
            assert_eq!(record.classification.is_human, Some(true));
        }
    }
    assert!(cursor(&flagged.0, NATIVE));

    // A replay reads the session whole again: the record is already
    // indexed, so the proof abstains and nothing changes.
    let mut replayed = flagged.0;
    let again = flagged_into(&mut replayed, ok(&lines), clean);
    let claims = &again.origin.as_ref().unwrap().claims;
    assert_eq!((claims.recorded, claims.abstained.not_inserted), (0, 1));
    assert_eq!(proofs(&replayed, NATIVE), [proof(NATIVE, 2, 2)]);

    // So does the first evidence read of history the index already holds.
    let mut indexed = plain.0;
    let late = flagged_into(&mut indexed, ok(&lines), clean);
    let claims = &late.origin.as_ref().unwrap().claims;
    assert_eq!((claims.recorded, claims.abstained.not_inserted), (0, 1));
    assert!(proofs(&indexed, NATIVE).is_empty());
    for record in indexed.records(&format!("codex-{NATIVE}")).unwrap() {
        assert_eq!(record.human_excluded, record.uuid == uuid(2));
        if record.uuid == uuid(2) {
            assert_eq!(record.human_is_eligible, Some(false));
            assert_eq!(record.text_len, Some(SKILL_BODY.chars().count() as i64));
            assert!(record.content_json.is_none());
        }
    }
    assert!(
        indexed
            .records(&format!("codex-{NATIVE}"))
            .unwrap()
            .iter()
            .all(|record| !record.confirmed_automated_input)
    );
}

#[test]
fn a_later_line_disputing_a_claim_writes_it_without_proof_where_the_ordinary_import_has_it() {
    let mut malformed = claim(NATIVE, 5, 50);
    malformed["version"] = json!(2);
    let named = |key: &str, value: Value| {
        let mut other = claim(NATIVE, 5, 50);
        other[key] = value;
        other
    };
    let cases: Vec<(&str, Vec<Value>)> = vec![
        (
            "the same UUID again, with other content",
            vec![skill(2), assistant(3), user(2, "a different text")],
        ),
        (
            "the same UUID with a malformed claim",
            vec![skill(2), assistant(3), with(user(2, SKILL_BODY), malformed)],
        ),
        (
            "a valid claim of the same item",
            vec![
                skill(2),
                with(user(5, SKILL_BODY), named("item_id", json!(item(2)))),
            ],
        ),
        (
            "a valid claim of the same row",
            vec![
                skill(2),
                with(
                    user(5, SKILL_BODY),
                    named("row", json!({"index":2,"ordinal":null})),
                ),
            ],
        ),
        (
            "a malformed claim naming the same item",
            vec![skill(2), {
                let mut other = named("item_id", json!(item(2)));
                other["kind"] = json!("something_else");
                with(user(5, "text"), other)
            }],
        ),
        (
            "a claim naming the same record from another line",
            vec![
                skill(2),
                with(user(5, "text"), named("record_uuid", json!(uuid(2)))),
            ],
        ),
        (
            "an earlier refused claim naming the same item",
            vec![
                {
                    let mut other = named("item_id", json!(item(2)));
                    other["version"] = json!("1");
                    with(user(5, "text"), other)
                },
                skill(2),
            ],
        ),
        (
            "a dropped line naming the same item",
            vec![
                skill(2),
                with(
                    json!({"type":"user","timestamp":stamp(6),
                        "message":{"role":"user","content":"no identity"}}),
                    named("item_id", json!(item(2))),
                ),
            ],
        ),
    ];
    for (case, records) in cases {
        let lines = session(NATIVE, true, &records);
        let (plain, flagged) = both(&lines, clean);
        assert_parity(&plain, &flagged, &[NATIVE]);
        let claims = &flagged.1.origin.as_ref().unwrap().claims;
        assert_eq!(claims.disputed, 1, "{case}: {claims:?}");
        assert_eq!(claims.recorded, 0, "{case}");
        assert!(proofs(&flagged.0, NATIVE).is_empty(), "{case}");
    }
}

#[test]
fn a_dispute_after_its_claims_batch_committed_keeps_the_ordinary_write_order() {
    // The claim's batch commits before the repeat arrives: the disputed
    // record is written ahead of the repeat, as the ordinary import wrote it,
    // so the stored row keeps the claim line's content.
    let mut records = vec![skill(2)];
    records.extend(filler(1_000, MAX as u64 + 500));
    records.push(user(2, "a different text"));
    // A dispute inside the second batch returns the record to its place.
    records.push(skill(3));
    records.extend(filler(10_000, 10));
    records.push(user(3, "another text"));
    let lines = session(NATIVE, true, &records);
    let (plain, flagged) = both(&lines, clean);
    assert_parity(&plain, &flagged, &[NATIVE]);
    let claims = &flagged.1.origin.as_ref().unwrap().claims;
    assert_eq!((claims.claimed, claims.disputed), (2, 2), "{claims:?}");
    assert!(proofs(&flagged.0, NATIVE).is_empty());
}

#[test]
fn claims_beyond_one_batch_are_written_in_order_without_proof() {
    let over = 3;
    let records = (1..=(MAX + over) as u64).map(skill).collect::<Vec<_>>();
    let lines = session(NATIVE, true, &records);
    let (plain, flagged) = both(&lines, clean);
    assert_parity(&plain, &flagged, &[NATIVE]);
    let claims = &flagged.1.origin.as_ref().unwrap().claims;
    assert_eq!(
        (claims.claimed, claims.recorded, claims.over_cap),
        (MAX + over, MAX, over)
    );
    assert_eq!(proofs(&flagged.0, NATIVE).len(), MAX);
    assert!(cursor(&flagged.0, NATIVE));
}

#[test]
fn every_early_end_commits_exactly_the_ordinary_import_s_records() {
    // Claims in the first batch, the second batch and the tail; a second
    // session follows. Each end is taken where it can land.
    let claimed = [10u64, MAX as u64 + 100, 2 * MAX as u64 + 100];
    let records = |end: Option<(u64, Value)>| {
        let mut records = Vec::new();
        for n in 0..(2 * MAX as u64 + 500) {
            if let Some((at, line)) = &end
                && *at == n
            {
                records.push(line.clone());
                continue;
            }
            records.push(if claimed.contains(&n) {
                skill(n)
            } else if n % 2 == 0 {
                user(n, &format!("turn {n}"))
            } else {
                assistant(n)
            });
        }
        records
    };
    let then = |lines: &mut Vec<String>| {
        lines.extend(session(
            OTHER,
            true,
            &[with(user(90_000, SKILL_BODY), claim(OTHER, 90_000, 1))],
        ))
    };
    // A label the writer refuses fails the batch that holds it.
    let refused = |n: u64| {
        let mut record = assistant(n);
        record["source_surface"] = json!("elsewhere");
        record
    };
    let mut malformed = session(
        NATIVE,
        true,
        &records(Some((2 * MAX as u64 + 200, json!({"type":"system"})))),
    );
    then(&mut malformed);
    let mid = MAX as u64 + 300;
    let mut mid_batch = session(NATIVE, true, &records(Some((mid, refused(mid)))));
    then(&mut mid_batch);
    let late = 2 * MAX as u64 + 300;
    let mut tail = session(NATIVE, true, &records(Some((late, refused(late)))));
    then(&mut tail);
    let whole = session(NATIVE, true, &records(None));

    for (case, lines, finish, unwritten) in [
        ("malformed record", malformed, clean as Finish, 3),
        ("writer error in a full batch", mid_batch, clean, 3),
        ("writer error in the tail", tail, clean, 3),
        ("reader failure", whole.clone(), failed, 3),
        ("cancellation", whole.clone(), cancelled, 3),
    ] {
        let (plain, flagged) = both(&lines, finish);
        assert_parity(&plain, &flagged, &[NATIVE, OTHER]);
        let claims = &flagged.1.origin.as_ref().unwrap().claims;
        assert_eq!(claims.unwritten, unwritten, "{case}: {claims:?}");
        assert!(proofs(&flagged.0, NATIVE).is_empty(), "{case}");
        assert!(
            matches!(
                flagged.1.sessions[0].outcome,
                SessionOutcome::Skipped { .. }
            ),
            "{case}"
        );
        // The first batch's claim is stored, as the ordinary import stores it.
        let stored = ordinary(&flagged.0, &[NATIVE]);
        assert!(stored.iter().any(|r| r.uuid == uuid(10)), "{case}");
        if lines.len() > whole.len() {
            // The next session still imports with its evidence.
            assert_eq!(proofs(&flagged.0, OTHER).len(), 1, "{case}");
        }
    }

    // A read error mid-stream drops what the open batch held.
    let mut lines = ok(&whole[..(2 * MAX + 200)]);
    lines.push(Err(std::io::Error::other("pipe broke")));
    let mut plain = Store::open_in_memory().unwrap();
    let plain_lines = ok(&without_evidence(&whole[..(2 * MAX + 200)]))
        .into_iter()
        .chain(std::iter::once(Err(std::io::Error::other("pipe broke"))))
        .collect();
    let plain_report = plain_into(&mut plain, plain_lines, clean);
    let mut flagged = Store::open_in_memory().unwrap();
    let flagged_report = flagged_into(&mut flagged, lines, clean);
    assert_eq!(flagged_report.status, HostStatus::ReaderFailed);
    assert_parity(
        &(plain, plain_report),
        &(flagged, flagged_report),
        &[NATIVE],
    );
}

#[test]
fn a_record_the_writer_refuses_stays_in_its_batch_even_when_claimed() {
    let mut record = user(2, SKILL_BODY);
    record["source_surface"] = json!("elsewhere");
    let mut records = filler(100, 10);
    records.push(with(record, claim(NATIVE, 2, 2)));
    records.extend(filler(200, MAX as u64));
    let lines = session(NATIVE, true, &records);
    let (plain, flagged) = both(&lines, clean);
    assert_parity(&plain, &flagged, &[NATIVE]);
    let claims = &flagged.1.origin.as_ref().unwrap().claims;
    assert_eq!((claims.claimed, claims.unholdable), (1, 1));
    // The claim's batch failed, as it does without evidence: nothing of it
    // stays, and the session ends there.
    assert_eq!(flagged.0.counts().unwrap().records, 0);
    assert!(matches!(
        flagged.1.sessions[0].outcome,
        SessionOutcome::Skipped { .. }
    ));
}

/// A marked session whose header leaves open every label a record line may
/// also carry: surface, native start, cwd and branch.
fn open_session(records: &[Value]) -> Vec<String> {
    let mut header: Value = serde_json::from_str(&header(NATIVE, true)).unwrap();
    for key in ["source_surface", "started_at", "cwd", "git_branch"] {
        header[key] = Value::Null;
    }
    std::iter::once(header.to_string())
        .chain(records.iter().map(Value::to_string))
        .collect()
}

fn labelled(mut record: Value, key: &str, value: Value) -> Value {
    record[key] = value;
    record
}

/// A claim whose record could fill, disagree or fail differently once moved
/// out of its batch stays where the ordinary import writes it, without
/// proof; every ordinary record and session fact comes out the same.
#[test]
fn a_claim_that_could_change_its_batch_or_session_is_not_held() {
    let surface = |record: Value, value: &str| labelled(record, "source_surface", json!(value));
    let start = |record: Value, value: &str| labelled(record, "started_at", json!(value));
    let cwd = |record: Value, value: &str| labelled(record, "cwd", json!(value));
    // Two windows that each agree, whose claims disagree once together.
    let mut combined = vec![surface(user(1, "ask"), "cli"), surface(skill(2), "cli")];
    combined.extend(filler(100, MAX as u64 - 2));
    combined.extend([
        surface(user(3, "ask"), "vscode"),
        surface(skill(4), "vscode"),
    ]);
    let cases: Vec<(&str, Vec<Value>, usize)> = vec![
        (
            "a surface the header lacks, disagreeing later in the batch",
            vec![surface(skill(2), "cli"), surface(user(3, "ask"), "vscode")],
            1,
        ),
        (
            "a native start the header lacks, disagreeing later in the batch",
            vec![
                start(skill(2), "2026-09-25T09:00:00Z"),
                start(user(3, "ask"), "2026-09-25T09:30:00Z"),
            ],
            1,
        ),
        (
            "surface evidence the Store refuses",
            vec![
                labelled(skill(2), "surface_evidence", json!({"source":"bad label"})),
                user(3, "ask"),
            ],
            1,
        ),
        (
            "claims of two agreeing windows that disagree together",
            combined,
            2,
        ),
        (
            "a cwd the header lacks, filled once by record order",
            vec![
                user(1, "ask"),
                cwd(skill(2), "/a"),
                cwd(user(5, "later"), "/b"),
            ],
            1,
        ),
        (
            "a branch the header lacks",
            vec![
                labelled(skill(2), "gitBranch", json!("feature")),
                labelled(user(5, "later"), "gitBranch", json!("main")),
            ],
            1,
        ),
    ];
    for (case, records, unholdable) in cases {
        let lines = open_session(&records);
        let (plain, flagged) = both(&lines, clean);
        assert_parity(&plain, &flagged, &[NATIVE]);
        let claims = &flagged.1.origin.as_ref().unwrap().claims;
        assert_eq!(
            (claims.claimed, claims.unholdable, claims.recorded),
            (unholdable, unholdable, 0),
            "{case}: {claims:?}"
        );
        assert!(proofs(&flagged.0, NATIVE).is_empty(), "{case}");
    }

    // The cwd case commits everything, and the session keeps the claim's
    // cwd, as the ordinary import fills it.
    let lines = open_session(&[
        user(1, "ask"),
        cwd(skill(2), "/a"),
        cwd(user(5, "later"), "/b"),
    ]);
    let (plain, flagged) = both(&lines, clean);
    let row = flagged
        .0
        .session(&format!("codex-{NATIVE}"))
        .unwrap()
        .unwrap();
    assert_eq!(
        (row.meta.cwd.as_deref(), row.has_conflict),
        (Some("/a"), true)
    );
    assert_eq!(plain.0.counts().unwrap().records, 3);
}

/// Labels a record shares with its header, or a cwd or branch the header
/// already fills first, cannot move anything: such a claim is still held
/// and recorded.
#[test]
fn a_claim_agreeing_with_its_header_is_still_recorded() {
    let cases: Vec<(&str, Value)> = vec![
        (
            "the header's own surface and start",
            labelled(
                labelled(skill(2), "source_surface", json!("codex_cli")),
                "started_at",
                json!("2026-09-25T10:00:00Z"),
            ),
        ),
        (
            "another cwd than the header's, which fills first",
            labelled(skill(2), "cwd", json!("/elsewhere")),
        ),
        (
            "a tool use block beside the body",
            with(
                json!({"type":"user","uuid":uuid(2),"timestamp":stamp(2),
                    "message":{"role":"user","content":[
                        {"type":"text","text":SKILL_BODY},
                        {"type":"tool_use","id":"toolu_1","name":"Bash",
                            "input":{"command":"ls"}}]}}),
                claim(NATIVE, 2, 2),
            ),
        ),
    ];
    for (case, record) in cases {
        let mut records = vec![user(1, "ask"), record];
        records.extend(filler(100, MAX as u64 + 10));
        records.push(labelled(user(3, "later"), "cwd", json!("/later")));
        let lines = session(NATIVE, true, &records);
        let (plain, flagged) = both(&lines, clean);
        assert_parity(&plain, &flagged, &[NATIVE]);
        let claims = &flagged.1.origin.as_ref().unwrap().claims;
        assert_eq!(
            (claims.claimed, claims.recorded, claims.unholdable),
            (1, 1, 0),
            "{case}: {claims:?}"
        );
        assert_eq!(proofs(&flagged.0, NATIVE), [proof(NATIVE, 2, 2)], "{case}");
    }
}

#[test]
fn a_proof_bearing_batch_the_store_refuses_is_written_again_without_proofs() {
    let lines = session(NATIVE, true, &[user(1, "ask"), user(2, SKILL_BODY)]);
    let mut events = StreamEvents::with_codex_origin_evidence();
    let mut header = None;
    let mut records = Vec::new();
    for line in &lines {
        match events.push(line).unwrap() {
            Some(StreamEvent::Session(parsed)) => header = Some(parsed),
            Some(StreamEvent::Record(record)) => records.push(*record),
            other => panic!("{other:?}"),
        }
    }
    let header = header.unwrap();
    let begin = |store: &mut Store| {
        SessionWriter::begin(store, Host::Codex, SessionSource::ReadersCli, &header, 7).unwrap()
    };
    // A proof naming another session: the Store refuses the batch whole.
    let mut store = Store::open_in_memory().unwrap();
    let mut writer = begin(&mut store);
    let refused = [None, Some(proof(OTHER, 2, 2))];
    assert!(matches!(
        writer.write_proven(&mut store, &records, &refused, 7),
        Ok(ProvenWrite::Refused)
    ));
    let mut plain = Store::open_in_memory().unwrap();
    let mut ordinary_writer = begin(&mut plain);
    ordinary_writer.write(&mut plain, &records, 7).unwrap();
    assert_eq!(ordinary(&store, &[NATIVE]), ordinary(&plain, &[NATIVE]));
    assert!(proofs(&store, NATIVE).is_empty());
    assert_eq!(
        writer.complete(&mut store, None),
        ordinary_writer.complete(&mut plain, None)
    );
    // The proof bound to its own session is kept.
    let mut store = Store::open_in_memory().unwrap();
    let mut writer = begin(&mut store);
    match writer.write_proven(&mut store, &records, &[None, Some(proof(NATIVE, 2, 2))], 7) {
        Ok(ProvenWrite::Kept(outcomes)) => assert_eq!(outcomes.len(), 1),
        _ => panic!("the proof is kept"),
    }
    assert_eq!(proofs(&store, NATIVE), [proof(NATIVE, 2, 2)]);
}

fn diagnostic(code: &str, native: Option<&str>) -> ReaderDiagnostic {
    ReaderDiagnostic {
        code: code.into(),
        path: native.map(path),
    }
}

fn reported(
    lines: &[String],
    diagnostics: Vec<ReaderDiagnostic>,
    complete: bool,
    origin: bool,
) -> HostReport {
    let mut store = Store::open_in_memory().unwrap();
    let finish = move || {
        Ok(ReaderOutcome {
            diagnostics,
            complete,
        })
    };
    if origin {
        import_reader_lines_origin(&mut store, "test".into(), ok(lines), 7, finish)
    } else {
        import_reader_lines(
            &mut store,
            Host::Codex,
            "test".into(),
            ok(&without_evidence(lines)),
            7,
            finish,
        )
    }
}

#[test]
fn a_withheld_notice_is_advisory_while_every_other_diagnostic_keeps_its_meaning() {
    let mut lines = session(NATIVE, true, &[skill(2)]);
    lines.extend(session(OTHER, false, &[user(7, "ask")]));
    let withheld = diagnostic("origin_evidence_withheld", Some(OTHER));
    let changed = diagnostic("source_changed", Some("/synthetic/elsewhere.jsonl"));

    let advisory = reported(&lines, vec![withheld.clone()], true, true);
    assert_eq!(advisory.status, HostStatus::Complete, "{advisory:?}");
    assert!(advisory.diagnostics.is_empty());
    let origin = advisory.origin.as_ref().unwrap();
    assert_eq!((origin.sessions.marked, origin.sessions.withheld), (1, 1));
    let notices = &origin.withheld_notices;
    assert_eq!(
        (
            notices.total,
            notices.unmatched,
            notices.marked_conflict,
            notices.unreported
        ),
        (1, 0, 0, 0)
    );
    // Without the flag the same code is an ordinary diagnostic.
    let plain = reported(&lines, vec![withheld.clone()], true, false);
    assert_eq!(plain.status, HostStatus::Incomplete);
    assert_eq!(plain.diagnostics, std::slice::from_ref(&withheld));

    // A real incompleteness stays one, with or beside the notice.
    let incomplete = reported(&lines, vec![withheld.clone(), changed.clone()], false, true);
    assert_eq!(incomplete.status, HostStatus::Incomplete);
    assert_eq!(incomplete.diagnostics, std::slice::from_ref(&changed));
    let exit_two = reported(&lines, vec![withheld.clone()], false, true);
    assert_eq!(exit_two.status, HostStatus::Incomplete);
    assert!(exit_two.diagnostics.is_empty());

    // Notices are checked against the headers: one naming a marked header, one
    // naming no header, and an unmarked header no notice names.
    let crossed = reported(
        &lines,
        vec![
            diagnostic("origin_evidence_withheld", Some(NATIVE)),
            diagnostic("origin_evidence_withheld", Some("/nowhere")),
            diagnostic("origin_evidence_withheld", None),
        ],
        true,
        true,
    );
    let notices = &crossed.origin.as_ref().unwrap().withheld_notices;
    assert_eq!(
        (
            notices.total,
            notices.unmatched,
            notices.marked_conflict,
            notices.unreported
        ),
        (3, 2, 1, 1)
    );
}

#[test]
fn the_ordinary_import_reports_no_origin_and_serializes_as_before() {
    let mut lines = session(NATIVE, false, &[user(1, "ask"), assistant(2)]);
    lines.extend(session(OTHER, false, &[user(3, "ask")]));
    let plain = reported(&lines, Vec::new(), true, false);
    assert_eq!(plain.origin, None);
    let json = serde_json::to_value(&plain).unwrap();
    let keys = json
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        ["detail", "diagnostics", "host", "sessions", "status"]
    );
    // The origin import of the same unmarked stream writes the same rows and
    // sessions, and says what it saw.
    let ((plain_store, plain_report), (flagged_store, flagged_report)) = both(&lines, clean);
    assert_eq!(
        ordinary(&flagged_store, &[NATIVE, OTHER]),
        ordinary(&plain_store, &[NATIVE, OTHER])
    );
    assert_eq!(flagged_report.sessions, plain_report.sessions);
    let flagged_json = serde_json::to_value(&flagged_report).unwrap();
    assert_eq!(flagged_json["origin"]["sessions"]["withheld"], 2);
    let mut stripped = flagged_json.as_object().unwrap().clone();
    stripped.remove("origin");
    assert_eq!(
        Value::Object(stripped),
        serde_json::to_value(&plain_report).unwrap()
    );
}

/// The per-reason counts of refused evidence, by fixed code only.
#[test]
fn refused_evidence_is_counted_by_fixed_code_and_the_record_still_imports() {
    let mut wrong = claim(NATIVE, 2, 2);
    wrong["kind"] = json!("codex_other");
    let lines = session(NATIVE, true, &[with(user(2, SKILL_BODY), wrong), skill(3)]);
    let (plain, flagged) = both(&lines, clean);
    assert_parity(&plain, &flagged, &[NATIVE]);
    let claims = &flagged.1.origin.as_ref().unwrap().claims;
    assert_eq!(
        claims.refused,
        BTreeMap::from([("origin_claim_kind", 1usize)])
    );
    assert_eq!(claims.recorded, 1);
    let rendered = serde_json::to_string(&flagged.1.origin).unwrap();
    for leaked in [SKILL_BODY, "/synthetic", "Review", NATIVE] {
        assert!(!rendered.contains(leaked), "{rendered}");
    }
}

/// The pinned producer itself, from the vendored bundle, over a synthetic
/// Codex home: two flat sessions, each with a person's `$skill` ask, the body
/// Codex injected beside it, and pasted text that looks like a skill body.
/// Read with and without `--origin-evidence`, the lines differ only by the
/// evidence, and the two imports store the same records and usage; only the
/// flagged one records a proof, one per injected body.
#[test]
fn conformance_pinned_producer_origin_import() {
    use std::{fs, path::Path};
    use xt_ingest::native::{
        ImportRequest, ProducerSource, import_native,
        readers_cli::{
            read_pin, resolve_python, spawn_codex_reader_with_origin_evidence, spawn_reader,
        },
    };
    let named = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    let Ok(python) = resolve_python(Some(named.as_os_str()), None) else {
        println!("SKIP conformance_pinned_producer_origin_import: Python 3.10+ is unavailable");
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
    let stamp = "2026-01-01T00:00:00Z";
    let entry =
        |kind: &str, payload: Value| json!({"timestamp":stamp,"type":kind,"payload":payload});
    let turn = |n: u64| format!("0e000000-0000-4000-8000-{n:012x}");
    let message = |n: u64| format!("msg_{n:050x}");
    let input = |item: String, turn: String, kinds: &[&str], text: &str| {
        entry(
            "response_item",
            json!({"type":"message","role":"user","id":item,
                "content":kinds.iter().map(|_| json!({"type":"input_text","text":text})).collect::<Vec<_>>(),
                "internal_chat_message_metadata_passthrough":
                    {"turn_id":turn,"create_time":1.0,"content_item_kinds":kinds}}),
        )
    };
    let sessions = [
        ("0a0a0a0a-1111-4222-8333-444444444444", "00", 0u64),
        ("0d0d0d0d-1111-4222-8333-444444444444", "09", 100u64),
    ];
    for (sid, second, base) in sessions {
        let mut rows = vec![entry(
            "session_meta",
            json!({"id":sid,"timestamp":stamp,"cwd":"/synthetic/project",
                "originator":"codex_cli","history_mode":"legacy"}),
        )];
        for (offset, pasted) in [(1u64, false), (2, true)] {
            let (t, n) = (turn(base + offset), base + offset * 10);
            rows.push(entry(
                "event_msg",
                json!({"type":"task_started","turn_id":t,"started_at":1,
                    "collaboration_mode_kind":"default","model_context_window":1}),
            ));
            rows.push(entry(
                "turn_context",
                json!({"model":"synthetic-model","turn_id":t}),
            ));
            if pasted {
                rows.push(input(message(n), t.clone(), &["user.text"], SKILL_BODY));
            } else {
                rows.push(input(
                    message(n),
                    t.clone(),
                    &["user.text"],
                    "$review please check this branch",
                ));
                rows.push(input(
                    message(n + 1),
                    t.clone(),
                    &["skills.selected_skill_instructions"],
                    SKILL_BODY,
                ));
            }
            rows.push(entry(
                "response_item",
                json!({"type":"message","role":"assistant","id":message(n + 2),
                    "content":[{"type":"output_text","text":"done"}]}),
            ));
            rows.push(entry(
                "event_msg",
                json!({"type":"token_count","info":{"total_token_usage":
                    {"input_tokens":10 * (base + offset),"output_tokens":1}}}),
            ));
            rows.push(entry(
                "event_msg",
                json!({"type":"task_complete","turn_id":t}),
            ));
        }
        let dir = home.join(".codex/sessions/2026/01/01");
        fs::create_dir_all(&dir).unwrap();
        let text = rows
            .iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>();
        fs::write(
            dir.join(format!("rollout-2026-01-01T00-00-{second}-{sid}.jsonl")),
            text,
        )
        .unwrap();
    }
    let natives = sessions.map(|(sid, _, _)| sid);
    let read = |flagged: bool| {
        let (stdout, handle) = if flagged {
            spawn_codex_reader_with_origin_evidence(&python, &producer, &home, None)
        } else {
            spawn_reader(&python, &producer, Host::Codex, &home, None)
        }
        .unwrap();
        let lines = std::io::BufRead::lines(stdout)
            .collect::<std::io::Result<Vec<_>>>()
            .unwrap();
        let outcome = handle.finish().unwrap();
        (lines, outcome)
    };
    let (plain_lines, plain_run) = read(false);
    let (flagged_lines, flagged_run) = read(true);
    assert!(plain_run.complete && plain_run.diagnostics.is_empty());
    assert!(flagged_run.complete && flagged_run.diagnostics.is_empty());
    // Line for line, the flagged export is the plain one plus its evidence:
    // a marker on each header and a claim on each injected body.
    let parsed = |line: &String| serde_json::from_str::<Value>(line).unwrap();
    let mut added = 0;
    let stripped = flagged_lines
        .iter()
        .map(|line| {
            let mut value = parsed(line);
            added += usize::from(
                value
                    .as_object_mut()
                    .unwrap()
                    .remove("origin_evidence")
                    .is_some(),
            );
            value
        })
        .collect::<Vec<_>>();
    assert_eq!(stripped, plain_lines.iter().map(parsed).collect::<Vec<_>>());
    assert_eq!(added, 4, "two markers and two claims");

    let mut plain = Store::open_in_memory().unwrap();
    let plain_report = import_reader_lines(
        &mut plain,
        Host::Codex,
        "pinned".into(),
        plain_lines.into_iter().map(Ok),
        7,
        clean,
    );
    let mut flagged = Store::open_in_memory().unwrap();
    let flagged_report = import_reader_lines_origin(
        &mut flagged,
        "pinned".into(),
        flagged_lines.into_iter().map(Ok),
        7,
        clean,
    );
    assert_eq!(
        plain_report.status,
        HostStatus::Complete,
        "{plain_report:?}"
    );
    assert_eq!(
        flagged_report.status,
        HostStatus::Complete,
        "{flagged_report:?}"
    );
    assert_eq!(flagged_report.sessions, plain_report.sessions);
    assert_eq!(ordinary(&flagged, &natives), ordinary(&plain, &natives));
    assert_eq!(
        session_rows(&flagged, &natives),
        session_rows(&plain, &natives)
    );
    assert_eq!(flagged.counts().unwrap(), plain.counts().unwrap());
    assert!(plain.counts().unwrap().records > 0 && plain.counts().unwrap().usage_rows > 0);
    let origin = flagged_report.origin.as_ref().unwrap();
    assert_eq!(origin.sessions.marked, 2, "{origin:?}");
    assert_eq!((origin.claims.claimed, origin.claims.recorded), (2, 2));
    assert_eq!(
        origin.claims.unholdable, 0,
        "the producer's claims are held"
    );
    assert_eq!(origin.withheld_notices.total, 0);
    for native in natives {
        let proven = proofs(&flagged, native);
        assert_eq!(proven.len(), 1, "{native}");
        let records = flagged.records(&format!("codex-{native}")).unwrap();
        // Only the injected body stops counting; the ask and the paste stay
        // the person's own.
        let confirmed = records
            .iter()
            .filter(|record| record.confirmed_automated_input)
            .map(|record| record.uuid.clone())
            .collect::<Vec<_>>();
        assert_eq!(confirmed, [proven[0].record_uuid.clone()]);
        let human = records
            .iter()
            .filter(|record| record.classification.is_human == Some(true))
            .count();
        assert_eq!(
            human, 3,
            "{native}: the ask, the body's raw class, the paste"
        );
    }

    // The scan itself reads Codex with its evidence.
    let mut scanned = Store::open_in_memory().unwrap();
    let report = import_native(
        &mut scanned,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Codex],
            producer: &source,
            python: Some(python.as_os_str()),
            observed_at: 7,
            cancel: None,
        },
    );
    let codex = &report.hosts[0];
    assert_eq!(codex.status, HostStatus::Complete, "{codex:?}");
    assert_eq!(
        codex.origin.as_ref().map(|origin| origin.claims.recorded),
        Some(2)
    );
    assert_eq!(ordinary(&scanned, &natives), ordinary(&plain, &natives));
    assert_eq!(
        session_rows(&scanned, &natives),
        session_rows(&plain, &natives)
    );
}

#[test]
fn unchanged_history_and_new_inputs_share_the_heartbeat_rule() {
    let heartbeat = "<heartbeat><automation_id>synthetic-timer</automation_id><current_time_iso>2026-09-25T10:00:00Z</current_time_iso><instructions>check status</instructions></heartbeat>";
    let lines = session(
        NATIVE,
        true,
        &[user(1, heartbeat), user(2, "I pasted a skill request")],
    );
    let mut store = Store::open_in_memory().unwrap();
    let first = flagged_into(&mut store, ok(&lines), clean);
    assert_eq!(first.origin.unwrap().human_inputs.applied, 1);
    let again = flagged_into(&mut store, ok(&lines), clean);
    assert_eq!(again.origin.unwrap().human_inputs.unchanged, 1);
    let appended = session(
        NATIVE,
        true,
        &[
            user(1, heartbeat),
            user(2, "I pasted a skill request"),
            user(3, heartbeat),
        ],
    );
    let next = flagged_into(&mut store, ok(&appended), clean);
    assert_eq!(next.origin.unwrap().human_inputs.applied, 1);
    let rows = store.records(&format!("codex-{NATIVE}")).unwrap();
    assert_eq!(rows.iter().filter(|r| r.human_excluded).count(), 2);
    assert!(
        rows.iter()
            .all(|r| !r.confirmed_automated_input && r.content_json.is_none())
    );
    assert_eq!(rows[1].human_is_eligible, Some(true));
    assert_eq!(rows[0].text_len, Some(heartbeat.chars().count() as i64));
}

/// Codex previews: a person's message keeps one; an input an adjustment says
/// is not (all) the person's never does, though the adjustment is applied
/// only after its batch commits; a proven injected body never does.
#[test]
fn previews_are_kept_only_for_the_persons_own_codex_inputs() {
    use xt_store::record_preview::{Preview, PreviewKind};
    let heartbeat = "<heartbeat><automation_id>synthetic-timer</automation_id><current_time_iso>2026-09-25T10:00:00Z</current_time_iso><instructions>check status</instructions></heartbeat>";
    let lines = session(
        NATIVE,
        true,
        &[
            user(1, heartbeat),
            user(2, "I pasted   a skill request"),
            skill(3),
        ],
    );
    let mut store = Store::open_in_memory().unwrap();
    for _ in 0..2 {
        assert_eq!(
            store.record_preview(&uuid(3), PreviewKind::Person).unwrap(),
            None
        );
        let report = flagged_into(&mut store, ok(&lines), clean);
        assert_eq!(report.status, HostStatus::Complete, "{report:?}");
        assert_eq!(
            store.record_preview(&uuid(2), PreviewKind::Person).unwrap(),
            Some(Preview {
                text: "I pasted a skill request".into(),
                truncated: false
            })
        );
        assert_eq!(
            store.record_preview(&uuid(1), PreviewKind::Person).unwrap(),
            None
        );
    }
}

/// A refused proof-bearing batch is written again without proofs; the first,
/// failed attempt must leave the writer's question memory exactly as it was,
/// so the retry still recognises the person's answer to the agent's question
/// and counts only the answer, as the ordinary write does.
#[test]
fn a_refused_attempt_leaves_the_writers_question_memory_unchanged() {
    let question = json!({"type":"assistant","uuid":uuid(1),"timestamp":stamp(1),
        "message":{"role":"assistant","content":[{"type":"tool_use","name":"request_user_input_async",
            "id":"call-q","input":{"questions":[{"title":"Choose?"}]}}]}});
    let answer = format!(
        "<send_user_message_question_reply>{}</send_user_message_question_reply>",
        json!([{"questionItemId":json!(["request_user_input_async","call-q",0]).to_string(),
            "question":"Choose?","answer":"yes"}])
    );
    let lines = session(
        NATIVE,
        true,
        &[question, user(2, &answer), user(3, SKILL_BODY)],
    );
    let mut events = StreamEvents::with_codex_origin_evidence();
    let mut header = None;
    let mut records = Vec::new();
    for line in &lines {
        match events.push(line).unwrap() {
            Some(StreamEvent::Session(parsed)) => header = Some(parsed),
            Some(StreamEvent::Record(record)) => records.push(*record),
            other => panic!("{other:?}"),
        }
    }
    let header = header.unwrap();
    let mut store = Store::open_in_memory().unwrap();
    let mut writer = SessionWriter::begin(
        &mut store,
        Host::Codex,
        SessionSource::ReadersCli,
        &header,
        7,
    )
    .unwrap();
    let refused = [None, None, Some(proof(OTHER, 3, 3))];
    assert!(matches!(
        writer.write_proven(&mut store, &records, &refused, 7),
        Ok(ProvenWrite::Refused)
    ));
    let reply = |store: &Store| {
        store
            .records(&format!("codex-{NATIVE}"))
            .unwrap()
            .into_iter()
            .find(|record| record.uuid == uuid(2))
            .unwrap()
    };
    let kept = reply(&store);
    assert_eq!(
        kept.human_text_len,
        Some(3),
        "only the answer is the person's"
    );
    assert_eq!(
        store
            .record_preview(&uuid(2), xt_store::record_preview::PreviewKind::Person)
            .unwrap(),
        None
    );
    let mut plain = Store::open_in_memory().unwrap();
    SessionWriter::begin(
        &mut plain,
        Host::Codex,
        SessionSource::ReadersCli,
        &header,
        7,
    )
    .unwrap()
    .write(&mut plain, &records, 7)
    .unwrap();
    assert_eq!(kept.human_text_len, reply(&plain).human_text_len);
}
