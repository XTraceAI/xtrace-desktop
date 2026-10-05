//! A structural automated-input confirmation against real ingestion paths: it
//! survives native replay and restart without changing identities, work or
//! sources, and it is part of the versioned measurement a capture receipt is
//! compared with, so a receipt cannot verify the corrected classification and
//! an older receipt is neither rewritten nor reinterpreted.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use xt_ingest::{
    canonical::{Parsed, ParsedRecord, SourceContext, parse_line},
    native::{ImportReport, ImportRequest, ProducerSource, import_native},
    writer::{WriteBatch, coverage, matches_current, write_batch},
};
use xt_store::{
    Host, SessionSource, Store,
    confirmation::{AutomatedInputProof, ConfirmationDisposition, EvidenceKind},
    ingest::{CaptureReceipt, RecordCoverage},
    measurement::{LEGACY_SCHEMA_VERSION, Projection, SCHEMA_VERSION},
    model::RecordIdentity,
};

const SID: &str = "00000000-0000-4000-8000-0000000000c1";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn proof(record_uuid: &str, session: &str, native: &str) -> AutomatedInputProof {
    AutomatedInputProof {
        record_uuid: record_uuid.into(),
        session_id: session.into(),
        native_session_id: native.into(),
        parent_host: Host::Codex,
        parent_session_id: "codex-synthetic-parent".into(),
        parent_tool_call_id: "call-synthetic".into(),
        parent_operation_index: 0,
        parent_result_id: None,
        evidence_kind: EvidenceKind::AgentDispatch,
        matcher_version: 1,
    }
}

fn native_home(home: &Path) -> PathBuf {
    let project = home.join(".claude/projects/-Users-synthetic-repo");
    fs::create_dir_all(&project).unwrap();
    let lines = fs::read_to_string(repo().join("fixtures/F1/input/sessions/session.jsonl"))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut record: Value = serde_json::from_str(line).unwrap();
            let object = record.as_object_mut().unwrap();
            object.insert("sessionId".into(), json!(SID));
            object.insert("entrypoint".into(), json!("cli"));
            record.to_string() + "\n"
        })
        .collect::<String>();
    let path = project.join(format!("{SID}.jsonl"));
    fs::write(&path, lines).unwrap();
    path
}

fn hashes(root: &Path) -> BTreeMap<PathBuf, String> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                let digest = format!("{:x}", Sha256::digest(fs::read(&path).unwrap()));
                out.insert(path, digest);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut out);
    out
}

fn import(store: &mut Store, home: &Path, observed_at: i64) -> ImportReport {
    import_native(
        store,
        &ImportRequest {
            home,
            hosts: &[Host::Claude],
            producer: &ProducerSource::Checkout {
                pin: repo().join(".plugin-pin"),
                plugin_root: None,
            },
            python: None,
            observed_at,
            cancel: None,
        },
    )
}

/// Every stored identity and work fact, plus the effective projection.
fn snapshot(path: &Path) -> (Vec<String>, Vec<String>) {
    let sql = rusqlite::Connection::open(path).unwrap();
    xt_store::timestamp::register_sqlite(&sql).unwrap();
    let rows = |query: &str| {
        sql.prepare(query)
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
    };
    let mut work = rows(
        "SELECT json_array(uuid,session_id,type,ts,api_message_id,request_id,role,model,
             is_tool_result_carrier,text_len,tool_use_count,content_json,has_conflict,is_human,parent_uuid)
         FROM records ORDER BY uuid",
    );
    work.extend(rows(
        "SELECT json_array(uuid,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens)
         FROM usage ORDER BY uuid",
    ));
    work.extend(rows(
        "SELECT json_array(uuid,block_index,name,kind,group_key) FROM tool_uses ORDER BY uuid,block_index",
    ));
    work.extend(rows(
        "SELECT json_array(session_id,native_session_id,record_count,first_ts,last_ts) FROM sessions",
    ));
    work.extend(rows(
        "SELECT json_array(uuid,input_tokens,output_tokens) FROM v_response_usage ORDER BY uuid",
    ));
    let effective = rows(
        "SELECT json_array(uuid,is_human,raw_is_human,confirmed_automated_input) FROM v_records ORDER BY uuid",
    );
    (work, effective)
}

#[test]
fn native_replay_and_restart_keep_the_confirmation_identities_work_and_sources() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let transcript = native_home(&home);
    let database = temp.path().join("index.sqlite");
    let sources = hashes(&home);
    let mut store = Store::open(&database).unwrap();
    assert!(import(&mut store, &home, 1_788_782_400_000).complete());
    let session = store.session(SID).unwrap().unwrap();
    assert_eq!(session.meta.native_session_id.as_deref(), Some(SID));
    let first_input = store
        .records(SID)
        .unwrap()
        .into_iter()
        .find(|r| r.classification.is_human == Some(true))
        .unwrap()
        .uuid;
    let (work, before) = snapshot(&database);

    let report = store
        .apply_automated_input_confirmations(&[proof(&first_input, SID, SID)], 1)
        .unwrap();
    assert_eq!(report.dispositions, [ConfirmationDisposition::Confirmed]);
    let (unchanged, corrected) = snapshot(&database);
    assert_eq!(unchanged, work);
    let changed: Vec<_> = before
        .iter()
        .zip(&corrected)
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(changed.len(), 1);
    assert_eq!(
        changed[0].1,
        &json!([first_input, 0, 1, 1]).to_string().replace(' ', "")
    );

    // A full native replay of the unchanged transcript, which has no idea the
    // input was automated, changes nothing.
    let sql = rusqlite::Connection::open(&database).unwrap();
    sql.execute("DELETE FROM native_checkpoints", []).unwrap();
    assert!(import(&mut store, &home, 1_788_782_400_001).complete());
    assert_eq!(snapshot(&database), (work.clone(), corrected.clone()));
    drop(store);

    // Restart, then replay an appended transcript: only the new record joins.
    let mut store = Store::open(&database).unwrap();
    assert_eq!(snapshot(&database), (work.clone(), corrected.clone()));
    let mut appended = fs::read_to_string(&transcript).unwrap();
    appended.push_str(
        &json!({"uuid":"33333333-3333-4333-8333-000000000001","type":"assistant","sessionId":SID,
            "entrypoint":"cli","timestamp":"2026-09-07T13:00:00Z",
            "message":{"role":"assistant","model":"fixture-model-v1","content":[{"type":"text","text":"Synthetic"}]}})
        .to_string(),
    );
    appended.push('\n');
    fs::write(&transcript, appended).unwrap();
    let grown = hashes(&home);
    assert!(import(&mut store, &home, 1_788_782_400_002).complete());
    assert_eq!(store.counts().unwrap().records, 26);
    let effective = snapshot(&database).1;
    assert!(
        effective.contains(
            &corrected
                .iter()
                .find(|r| r.contains(&first_input))
                .unwrap()
                .clone()
        )
    );
    assert_eq!(
        store.automated_input_confirmations(SID).unwrap(),
        [proof(&first_input, SID, SID)]
    );
    // Originals are read, never written: the only change is the test's own append.
    assert_eq!(hashes(&home), grown);
    assert_eq!(sources.len(), grown.len());
}

fn parsed(value: Value) -> ParsedRecord {
    match parse_line(&value.to_string()).unwrap() {
        Parsed::Record(record) => *record,
        _ => panic!("expected synthetic record"),
    }
}

#[test]
fn receipts_of_both_versions_stop_matching_a_corrected_input_and_are_never_rewritten() {
    let db_dir = tempfile::TempDir::new().unwrap();
    let mut store = Store::open(db_dir.path().join("receipts.sqlite")).unwrap();
    let context = SourceContext {
        conversation_id: Some("cursor-synthetic-native".into()),
        native_session_id: Some("synthetic-native".into()),
        source_platform: Some("cursor".into()),
        source_surface: Some("cli".into()),
        source: Some(SessionSource::Plugin),
        ..Default::default()
    };
    let records = [
        parsed(
            json!({"uuid":"input","type":"user","timestamp":"2026-09-07T12:00:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic dispatched"}]}}),
        ),
        parsed(
            json!({"uuid":"agent","type":"assistant","timestamp":"2026-09-07T12:01:00Z",
            "message":{"id":"msg","role":"assistant","model":"synthetic-model",
                "content":[{"type":"text","text":"Synthetic"}],
                "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
        ),
    ];
    let receipt = CaptureReceipt {
        receipt_id: "capture".into(),
        session_id: "cursor-synthetic-native".into(),
        surface: Some("cli".into()),
        received_at: 100,
    };
    write_batch(
        &mut store,
        &WriteBatch {
            namespace: None,
            context: &context,
            declared_host: None,
            records: &records,
            hook_summaries: &[],
            pr_witnesses: &[],
            title: None,
            cwd: None,
            git_branch: None,
            keep_content: false,
            observed_at: 100,
            receipt: Some(&receipt),
            cursor: None,
            discovery: None,
            checkpoint: None,
        },
    )
    .unwrap();
    let sealed = store.capture_coverage("capture").unwrap();
    assert!(
        sealed
            .iter()
            .all(|c| c.digest_schema_version == SCHEMA_VERSION)
    );
    let stored = |store: &Store, uuid: &str| {
        store
            .records("cursor-synthetic-native")
            .unwrap()
            .into_iter()
            .find(|r| r.uuid == uuid)
            .unwrap()
    };
    // What an older build sealed for the same records: version 1 bytes of the
    // very same measurement, which this build still recognises unchanged.
    let legacy = |store: &Store, uuid: &str| {
        let (mask, bytes) = Projection::from_stored(&stored(store, uuid))
            .unwrap()
            .encoded(LEGACY_SCHEMA_VERSION)
            .unwrap();
        RecordCoverage {
            record_uuid: uuid.into(),
            metric_field_mask: mask,
            measurement_revision: format!("{:x}", Sha256::digest(bytes)),
            digest_schema_version: LEGACY_SCHEMA_VERSION,
        }
    };
    let legacy_input = legacy(&store, "input");
    let legacy_agent = legacy(&store, "agent");
    let unknown_version = RecordCoverage {
        digest_schema_version: 3,
        ..sealed[1].clone()
    };
    for item in &sealed {
        assert!(matches_current(item, &stored(&store, &item.record_uuid)).unwrap());
    }
    assert!(matches_current(&legacy_input, &stored(&store, "input")).unwrap());
    assert!(matches_current(&legacy_agent, &stored(&store, "agent")).unwrap());
    assert!(!matches_current(&unknown_version, &stored(&store, "input")).unwrap());
    let usage_before = store.records("cursor-synthetic-native").unwrap()[0]
        .usage
        .clone();

    let report = store
        .apply_automated_input_confirmations(
            &[proof(
                "input",
                "cursor-synthetic-native",
                "synthetic-native",
            )],
            200,
        )
        .unwrap();
    assert_eq!(report.dispositions, [ConfirmationDisposition::Confirmed]);

    // The corrected input verifies under neither version; the untouched agent
    // record still verifies under both. The sealed receipt itself is unchanged.
    let input = stored(&store, "input");
    assert!(input.confirmed_automated_input);
    assert!(!matches_current(&sealed[1], &input).unwrap());
    assert!(!matches_current(&legacy_input, &input).unwrap());
    assert!(matches_current(&sealed[0], &stored(&store, "agent")).unwrap());
    assert!(matches_current(&legacy_agent, &stored(&store, "agent")).unwrap());
    assert_eq!(store.capture_coverage("capture").unwrap(), sealed);
    assert_eq!(
        store.records("cursor-synthetic-native").unwrap()[0].usage,
        usage_before
    );
    // A replayed capture of the same payload computes the incoming measurement,
    // which cannot state the confirmation, so it cannot witness the correction.
    assert_ne!(coverage("cursor-synthetic-native", &records[0]).unwrap(), {
        let (mask, bytes) = Projection::from_stored(&input)
            .unwrap()
            .encoded(SCHEMA_VERSION)
            .unwrap();
        RecordCoverage {
            record_uuid: "input".into(),
            metric_field_mask: mask,
            measurement_revision: format!("{:x}", Sha256::digest(bytes)),
            digest_schema_version: SCHEMA_VERSION,
        }
    });
    // The reader and ingest projections still agree for every unconfirmed row.
    assert_eq!(
        coverage("cursor-synthetic-native", &records[1]).unwrap(),
        sealed[0]
    );
}

#[test]
fn legacy_golden_revision_is_still_what_version_one_computes() {
    let mut store = Store::open_in_memory().unwrap();
    let context = SourceContext {
        conversation_id: Some("cursor-native".into()),
        native_session_id: Some("native".into()),
        source_platform: Some("cursor".into()),
        source_surface: Some("cli".into()),
        source: Some(SessionSource::ReadersCli),
        ..Default::default()
    };
    let records = [parsed(
        json!({"uuid":"row","type":"assistant","message":{}}),
    )];
    write_batch(
        &mut store,
        &WriteBatch {
            namespace: None,
            context: &context,
            declared_host: None,
            records: &records,
            hook_summaries: &[],
            pr_witnesses: &[],
            title: None,
            cwd: None,
            git_branch: None,
            keep_content: false,
            observed_at: 100,
            receipt: None,
            cursor: None,
            discovery: None,
            checkpoint: None,
        },
    )
    .unwrap();
    let row = store.records("cursor-native").unwrap().remove(0);
    // The version 1 golden the previous build pinned for this measurement.
    let sealed_by_older_build = RecordCoverage {
        record_uuid: "row".into(),
        metric_field_mask: 199,
        measurement_revision: "0c299a8f1b8c32e9d61ebc26bedf0da9ac99dad88e889cce513f15698afcc795"
            .into(),
        digest_schema_version: 1,
    };
    assert!(matches_current(&sealed_by_older_build, &row).unwrap());
    // The same digest under the current version's label is not accepted.
    assert!(
        !matches_current(
            &RecordCoverage {
                digest_schema_version: SCHEMA_VERSION,
                ..sealed_by_older_build
            },
            &row
        )
        .unwrap()
    );
}

#[test]
fn an_unchanged_retry_of_a_version_one_receipt_stays_idempotent() {
    let mut store = Store::open_in_memory().unwrap();
    let context = SourceContext {
        conversation_id: Some("cursor-synthetic-native".into()),
        native_session_id: Some("synthetic-native".into()),
        source_platform: Some("cursor".into()),
        source_surface: Some("cli".into()),
        source: Some(SessionSource::Plugin),
        ..Default::default()
    };
    let agent = |output_tokens: i64| {
        parsed(
            json!({"uuid":"agent","type":"assistant","timestamp":"2026-09-07T12:01:00Z",
            "message":{"id":"msg","role":"assistant","model":"synthetic-model",
                "content":[{"type":"text","text":"Synthetic"}],
                "usage":{"input_tokens":10,"output_tokens":output_tokens,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
        )
    };
    let records = [
        parsed(
            json!({"uuid":"input","type":"user","timestamp":"2026-09-07T12:00:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic dispatched"}]}}),
        ),
        agent(5),
    ];
    let receipt = |receipt_id: &str| CaptureReceipt {
        receipt_id: receipt_id.into(),
        session_id: "cursor-synthetic-native".into(),
        surface: Some("cli".into()),
        received_at: 100,
    };
    let submit = |store: &mut Store, receipt: &CaptureReceipt, records: &[ParsedRecord]| {
        write_batch(
            store,
            &WriteBatch {
                namespace: None,
                context: &context,
                declared_host: None,
                records,
                hook_summaries: &[],
                pr_witnesses: &[],
                title: None,
                cwd: None,
                git_branch: None,
                keep_content: false,
                observed_at: 100,
                receipt: Some(receipt),
                cursor: None,
                discovery: None,
                checkpoint: None,
            },
        )
    };
    submit(&mut store, &receipt("current"), &records).unwrap();
    let current = store.capture_coverage("current").unwrap();
    // What an older build sealed for the same submission: the version 1 bytes
    // of the incoming measurement, never of the stored record.
    let sealed_as = |version: u32| {
        records
            .iter()
            .map(|record| {
                let identity = RecordIdentity {
                    parent_uuid: record.native.parent_uuid.clone(),
                    agent_id: record.native.agent_id.clone(),
                    subtype: record.native.subtype.clone(),
                    first_seen_at: None,
                };
                let (mask, bytes) = Projection::from_canonical(
                    "cursor-synthetic-native",
                    &record.canonical,
                    &identity,
                )
                .unwrap()
                .encoded(LEGACY_SCHEMA_VERSION)
                .unwrap();
                RecordCoverage {
                    record_uuid: record.canonical.uuid.clone().unwrap(),
                    metric_field_mask: mask,
                    measurement_revision: format!("{:x}", Sha256::digest(bytes)),
                    digest_schema_version: version,
                }
            })
            .collect::<Vec<_>>()
    };
    let mut legacy = sealed_as(LEGACY_SCHEMA_VERSION);
    legacy.sort_by(|a, b| a.record_uuid.cmp(&b.record_uuid));
    store
        .insert_capture_receipt(&receipt("legacy"), &legacy)
        .unwrap();
    // A version this build does not know, with otherwise plausible facts.
    let future = sealed_as(3);
    store
        .insert_capture_receipt(&receipt("future"), &future)
        .unwrap();
    let rows = store.records("cursor-synthetic-native").unwrap();

    // The same payload retried under either version is acknowledged and adds
    // or rewrites nothing.
    let replayed = submit(&mut store, &receipt("legacy"), &records).unwrap();
    assert_eq!(replayed.ack_through.as_deref(), Some("agent"));
    assert_eq!((replayed.records_new, replayed.records_enriched), (0, 0));
    submit(&mut store, &receipt("current"), &records).unwrap();
    assert_eq!(store.capture_coverage("legacy").unwrap(), legacy);
    assert_eq!(store.capture_coverage("current").unwrap(), current);

    // A differing payload does not match either sealed version, and an unknown
    // version never matches; each fails atomically.
    let changed = [records[0].clone(), agent(6)];
    for receipt_id in ["legacy", "current"] {
        let err = submit(&mut store, &receipt(receipt_id), &changed).unwrap_err();
        assert!(err.to_string().contains("receipt retry does not match"));
    }
    let err = submit(&mut store, &receipt("future"), &records).unwrap_err();
    assert!(err.to_string().contains("receipt retry does not match"));
    assert_eq!(store.records("cursor-synthetic-native").unwrap(), rows);
    assert_eq!(store.capture_coverage("legacy").unwrap(), legacy);

    // After a confirmation the unchanged retry is still idempotent, but neither
    // sealed receipt verifies the corrected input; the agent record still does.
    store
        .apply_automated_input_confirmations(
            &[proof(
                "input",
                "cursor-synthetic-native",
                "synthetic-native",
            )],
            200,
        )
        .unwrap();
    submit(&mut store, &receipt("legacy"), &records).unwrap();
    submit(&mut store, &receipt("current"), &records).unwrap();
    assert_eq!(store.capture_coverage("legacy").unwrap(), legacy);
    assert_eq!(store.capture_coverage("current").unwrap(), current);
    let stored = store.records("cursor-synthetic-native").unwrap();
    let find = |uuid: &str| stored.iter().find(|row| row.uuid == uuid).unwrap();
    assert!(find("input").confirmed_automated_input);
    for sealed in [&legacy, &current] {
        let by_uuid = |uuid: &str| sealed.iter().find(|c| c.record_uuid == uuid).unwrap();
        assert!(!matches_current(by_uuid("input"), find("input")).unwrap());
        assert!(matches_current(by_uuid("agent"), find("agent")).unwrap());
    }
}
