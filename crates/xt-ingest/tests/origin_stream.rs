//! Opt-in Codex origin evidence on the shared reader stream. Synthetic lines
//! only: every claim is judged beside its record, and no judgement ever ends,
//! skips or changes a session or record. The ordinary stream is unchanged.

use serde_json::{Value, json};
use xt_ingest::{
    canonical::ParsedRecord,
    native::{
        origin::{Origin, OriginInvalid, OriginMarker, RecordOrigin, SessionOrigin},
        stream::{StreamEvent, StreamEvents},
    },
};
use xt_store::{
    Host, SessionSource,
    injected::{InjectedContextKind, InjectedContextProof, OriginContract, SegmentHistory},
};

const NATIVE: &str = "019a0000-0000-7000-8000-000000000001";
const OTHER: &str = "019a0000-0000-7000-8000-000000000002";
const TURN: &str = "019a0000-0000-7000-8000-0000000000f1";
const ROLLOUT: &str = "019a0000-0000-7000-8000-0000000000e1";
const CONTRACT: &str = "memhub.codex.origin_evidence";
const KIND: &str = "codex_selected_skill_instructions";
const TS: &str = "2026-09-25T10:00:00Z";
const SKILL_BODY: &str = "<skill>\n# Review\nFollow these steps.\n</skill>";
const HEADER_MISMATCH: &str = "session header does not match the shared stream contract";

fn uuid(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

fn item(n: u64) -> String {
    format!("msg_00000000-0000-4000-9000-{n:012x}")
}

fn hex_item() -> String {
    format!(
        "msg_{}",
        "0123456789abcdef0123456789abcdef0123456789abcdef01"
    )
}

fn header(native: &str, marker: Option<Value>) -> String {
    let mut header = json!({"type":"session","host":"codex","native_session_id":native,
        "conversation_id":format!("codex-{native}"),"source_surface":null,"started_at":null,
        "cwd":null,"git_branch":null,"title":null,"path":"/synthetic/rollout.jsonl","mtime":1.0});
    if let Some(marker) = marker {
        header["origin_evidence"] = marker;
    }
    header.to_string()
}

fn marker() -> Value {
    json!({"contract":CONTRACT,"version":1})
}

fn user(n: u64, text: &str) -> Value {
    json!({"type":"user","uuid":uuid(n),"timestamp":TS,"message":{"role":"user","content":text}})
}

fn assistant(n: u64) -> Value {
    json!({"type":"assistant","uuid":uuid(n),"timestamp":TS,
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

fn proof(n: u64, row: u32) -> InjectedContextProof {
    InjectedContextProof {
        contract: OriginContract::CodexOriginEvidence,
        version: 1,
        kind: InjectedContextKind::CodexSelectedSkillInstructions,
        native_session_id: NATIVE.into(),
        history: SegmentHistory::Flat,
        rollout_id: None,
        row_index: row,
        row_ordinal: None,
        item_id: item(n),
        turn_id: TURN.into(),
        record_uuid: uuid(n),
    }
}

fn lines(header: String, records: &[Value]) -> Vec<String> {
    std::iter::once(header)
        .chain(records.iter().map(Value::to_string))
        .collect()
}

fn marked(records: &[Value]) -> Vec<String> {
    lines(header(NATIVE, Some(marker())), records)
}

/// Every event of the lines in evidence mode, with its sidecar.
fn evidence(lines: &[String]) -> Vec<(StreamEvent, Option<Origin>)> {
    let mut stream = StreamEvents::with_codex_origin_evidence();
    lines
        .iter()
        .filter_map(|line| stream.push_with_origin(line).unwrap())
        .collect()
}

/// The sidecar of each line after the header, each of which must still be an
/// imported record.
fn record_origins(lines: &[String]) -> Vec<RecordOrigin> {
    let events = evidence(lines);
    assert_eq!(events.len(), lines.len(), "a line was skipped: {events:?}");
    assert!(matches!(
        events.first(),
        Some((StreamEvent::Session(_), Some(Origin::Session(_))))
    ));
    events
        .into_iter()
        .skip(1)
        .map(|event| match event {
            (StreamEvent::Record(_), Some(Origin::Record(origin))) => origin,
            other => panic!("expected an imported record with its origin, got {other:?}"),
        })
        .collect()
}

fn session_origin(lines: &[String]) -> (OriginMarker, SessionOrigin) {
    match evidence(lines).into_iter().next() {
        Some((StreamEvent::Session(header), Some(Origin::Session(origin)))) => {
            (header.origin_evidence, origin)
        }
        other => panic!("expected a session, got {other:?}"),
    }
}

fn parsed(events: Vec<(StreamEvent, Option<Origin>)>) -> Vec<ParsedRecord> {
    events
        .into_iter()
        .filter_map(|(event, _)| match event {
            StreamEvent::Record(record) => Some(*record),
            _ => None,
        })
        .collect()
}

#[test]
fn a_marked_flat_session_claims_only_the_injected_body() {
    let lines = marked(&[
        // The person's own `$skill` ask beside the injected body.
        user(1, "$review please check this branch"),
        with(user(2, SKILL_BODY), claim(NATIVE, 2, 5)),
        // Pasted text that looks like a skill body, even one quoting a claim,
        // carries no claim of its own.
        user(
            3,
            &format!(
                "{SKILL_BODY} {}",
                json!({"origin_evidence": claim(NATIVE, 3, 6)})
            ),
        ),
        assistant(4),
    ]);
    assert_eq!(
        session_origin(&lines),
        (OriginMarker::V1, SessionOrigin::Marked)
    );
    assert_eq!(
        record_origins(&lines),
        [
            RecordOrigin::Declined,
            RecordOrigin::Claimed(Box::new(proof(2, 5))),
            RecordOrigin::Declined,
            RecordOrigin::Declined,
        ]
    );
}

#[test]
fn a_marked_paginated_session_claims_a_continuation_row() {
    let mut paginated = claim(NATIVE, 2, 40);
    paginated["segment"] = json!({"history":"paginated","rollout_id":ROLLOUT});
    paginated["row"] = json!({"index":40,"ordinal":7});
    paginated["item_id"] = json!(hex_item());
    let lines = marked(&[user(1, "$review"), with(user(2, SKILL_BODY), paginated)]);
    assert_eq!(
        record_origins(&lines),
        [
            RecordOrigin::Declined,
            RecordOrigin::Claimed(Box::new(InjectedContextProof {
                history: SegmentHistory::Paginated,
                rollout_id: Some(ROLLOUT.into()),
                row_index: 40,
                row_ordinal: Some(7),
                item_id: hex_item(),
                ..proof(2, 40)
            })),
        ]
    );
}

/// A session the producer withheld (its stderr notice is not read here) is
/// written exactly as without the flag: no marker, so nothing is claimed, and
/// a stray claim is refused without touching the records.
#[test]
fn an_unmarked_session_is_withheld_and_claims_nothing() {
    let lines = lines(
        header(NATIVE, None),
        &[
            user(1, "$review"),
            with(user(2, SKILL_BODY), claim(NATIVE, 2, 5)),
            user(3, SKILL_BODY),
        ],
    );
    assert_eq!(
        session_origin(&lines),
        (OriginMarker::Absent, SessionOrigin::Withheld)
    );
    assert_eq!(
        record_origins(&lines),
        [
            RecordOrigin::Withheld,
            RecordOrigin::Invalid(OriginInvalid::Unmarked),
            RecordOrigin::Withheld,
        ]
    );
}

#[test]
fn a_wrong_marker_opens_the_session_but_claims_nothing() {
    for wrong in [
        json!(null),
        json!(true),
        json!([CONTRACT, 1]),
        json!({"contract":CONTRACT}),
        json!({"contract":CONTRACT,"version":2}),
        json!({"contract":CONTRACT,"version":"1"}),
        json!({"contract":CONTRACT,"version":1.0}),
        json!({"contract":CONTRACT,"version":1,"kind":KIND}),
        json!({"contract":"memhub.codex.source_witness","version":1}),
    ] {
        let lines = lines(
            header(NATIVE, Some(wrong.clone())),
            &[
                with(user(1, SKILL_BODY), claim(NATIVE, 1, 0)),
                user(2, "$review"),
            ],
        );
        assert_eq!(
            session_origin(&lines),
            (OriginMarker::Invalid, SessionOrigin::Invalid),
            "{wrong}"
        );
        assert_eq!(
            record_origins(&lines),
            [
                RecordOrigin::Invalid(OriginInvalid::SessionMarker),
                RecordOrigin::Invalid(OriginInvalid::SessionMarker),
            ],
            "{wrong}"
        );
    }
}

/// Each malformed claim fails its proof alone, with a fixed code. The record
/// imports, and the session keeps importing and claiming after it.
#[test]
fn a_malformed_claim_fails_only_its_proof() {
    fn set(key: &'static str, value: Value) -> impl Fn(&mut Value) {
        move |claim| claim[key] = value.clone()
    }
    fn remove(key: &'static str) -> impl Fn(&mut Value) {
        move |claim| {
            claim.as_object_mut().unwrap().remove(key);
        }
    }
    fn replace(value: Value) -> impl Fn(&mut Value) {
        move |claim| *claim = value.clone()
    }
    type Edit = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(Edit, OriginInvalid)> = vec![
        (
            Box::new(set("contract", json!("memhub.codex.source_witness"))),
            OriginInvalid::Contract,
        ),
        (Box::new(set("contract", json!(1))), OriginInvalid::Contract),
        (Box::new(set("version", json!(2))), OriginInvalid::Version),
        (Box::new(set("version", json!("1"))), OriginInvalid::Version),
        (Box::new(set("version", json!(1.0))), OriginInvalid::Version),
        (
            Box::new(set("version", json!(true))),
            OriginInvalid::Version,
        ),
        (
            Box::new(set("version", json!(null))),
            OriginInvalid::Version,
        ),
        (
            Box::new(set("kind", json!("codex_guardian_review"))),
            OriginInvalid::Kind,
        ),
        (Box::new(set("kind", json!(null))), OriginInvalid::Kind),
        (
            Box::new(set("native_session_id", json!(NATIVE.to_uppercase()))),
            OriginInvalid::Identifier,
        ),
        (
            Box::new(set("native_session_id", json!(""))),
            OriginInvalid::Identifier,
        ),
        (
            Box::new(set("item_id", json!(format!("rs_{}", &hex_item()[4..])))),
            OriginInvalid::Identifier,
        ),
        (
            Box::new(set("item_id", json!(&hex_item()[..53]))),
            OriginInvalid::Identifier,
        ),
        (
            Box::new(set("item_id", json!(null))),
            OriginInvalid::Identifier,
        ),
        (
            Box::new(set("turn_id", json!("turn-1"))),
            OriginInvalid::Identifier,
        ),
        (
            Box::new(set("record_uuid", json!("not-a-uuid"))),
            OriginInvalid::Identifier,
        ),
        (
            Box::new(set("record_uuid", json!(7))),
            OriginInvalid::Identifier,
        ),
        (
            Box::new(set(
                "segment",
                json!({"history":"flat","rollout_id":ROLLOUT}),
            )),
            OriginInvalid::Segment,
        ),
        (
            Box::new(set(
                "segment",
                json!({"history":"paginated","rollout_id":null}),
            )),
            OriginInvalid::Segment,
        ),
        (
            Box::new(set(
                "segment",
                json!({"history":"paginated","rollout_id":"ROLLOUT"}),
            )),
            OriginInvalid::Segment,
        ),
        (
            Box::new(set(
                "segment",
                json!({"history":"nested","rollout_id":null}),
            )),
            OriginInvalid::Segment,
        ),
        (
            Box::new(set("segment", json!({"history":"flat"}))),
            OriginInvalid::Segment,
        ),
        (
            Box::new(set(
                "segment",
                json!({"history":"flat","rollout_id":null,"path":"/x"}),
            )),
            OriginInvalid::Segment,
        ),
        (
            Box::new(set("segment", json!(null))),
            OriginInvalid::Segment,
        ),
        (
            Box::new(set("row", json!({"index":-1,"ordinal":null}))),
            OriginInvalid::Row,
        ),
        (
            Box::new(set("row", json!({"index":1.5,"ordinal":null}))),
            OriginInvalid::Row,
        ),
        (
            Box::new(set(
                "row",
                json!({"index":4_294_967_296_u64,"ordinal":null}),
            )),
            OriginInvalid::Row,
        ),
        (
            Box::new(set("row", json!({"index":"3","ordinal":null}))),
            OriginInvalid::Row,
        ),
        (
            Box::new(set("row", json!({"index":3,"ordinal":-1}))),
            OriginInvalid::Row,
        ),
        (
            Box::new(set("row", json!({"index":3,"ordinal":"7"}))),
            OriginInvalid::Row,
        ),
        (Box::new(set("row", json!({"index":3}))), OriginInvalid::Row),
        (
            Box::new(set("row", json!({"index":3,"ordinal":null,"byte_start":0}))),
            OriginInvalid::Row,
        ),
        (Box::new(set("row", json!(3))), OriginInvalid::Row),
        (
            Box::new(set("text", json!(SKILL_BODY))),
            OriginInvalid::Shape,
        ),
        (Box::new(set("sha256", json!("00"))), OriginInvalid::Shape),
        (Box::new(remove("contract")), OriginInvalid::Shape),
        (Box::new(remove("turn_id")), OriginInvalid::Shape),
        (Box::new(replace(json!(null))), OriginInvalid::Shape),
        (Box::new(replace(json!("claimed"))), OriginInvalid::Shape),
        (Box::new(replace(json!([]))), OriginInvalid::Shape),
        (Box::new(replace(json!({}))), OriginInvalid::Shape),
    ];
    for (index, (change, reason)) in cases.into_iter().enumerate() {
        let mut bad = claim(NATIVE, 1, 0);
        change(&mut bad);
        let lines = marked(&[
            with(user(1, SKILL_BODY), bad),
            user(2, "$review"),
            with(user(3, SKILL_BODY), claim(NATIVE, 3, 9)),
        ]);
        assert_eq!(
            record_origins(&lines),
            [
                RecordOrigin::Invalid(reason),
                RecordOrigin::Declined,
                RecordOrigin::Claimed(Box::new(proof(3, 9))),
            ],
            "case {index}"
        );
    }
}

#[test]
fn a_claim_binds_to_its_own_record_and_session() {
    let mut other_record = claim(NATIVE, 1, 0);
    other_record["record_uuid"] = json!(uuid(9));
    let lines = marked(&[
        with(user(1, SKILL_BODY), other_record),
        with(user(2, SKILL_BODY), claim(OTHER, 2, 1)),
        with(assistant(3), claim(NATIVE, 3, 2)),
    ]);
    assert_eq!(
        record_origins(&lines),
        [
            RecordOrigin::Invalid(OriginInvalid::RecordMismatch),
            RecordOrigin::Invalid(OriginInvalid::SessionMismatch),
            RecordOrigin::Invalid(OriginInvalid::NotUserRecord),
        ]
    );
    // A session under another header cannot carry this session's claim.
    let other = lines_for_other();
    assert_eq!(
        record_origins(&other),
        [RecordOrigin::Invalid(OriginInvalid::SessionMismatch)]
    );
}

fn lines_for_other() -> Vec<String> {
    lines(
        header(OTHER, Some(marker())),
        &[with(user(1, SKILL_BODY), claim(NATIVE, 1, 0))],
    )
}

/// Evidence is a Codex reader contract: the evidence stream is fixed to Codex
/// reader history, and another host's header is refused as before.
#[test]
fn evidence_mode_admits_only_codex_reader_sessions() {
    let mut cursor = serde_json::from_str::<Value>(&header(NATIVE, Some(marker()))).unwrap();
    cursor["host"] = json!("cursor");
    cursor["conversation_id"] = json!(format!("cursor-{NATIVE}"));
    let events = evidence(&[
        cursor.to_string(),
        with(user(1, SKILL_BODY), claim(NATIVE, 1, 0)).to_string(),
    ]);
    assert!(matches!(
        events.as_slice(),
        [(
            StreamEvent::MalformedHeader {
                reason: "session header names another host",
                ..
            },
            None
        )]
    ));
    let events = evidence(&marked(&[user(1, "$review")]));
    let [
        (StreamEvent::Session(_), _),
        (StreamEvent::Record(record), _),
    ] = events.as_slice()
    else {
        panic!("expected a session and its record, got {events:?}");
    };
    assert_eq!(record.context.source, Some(SessionSource::ReadersCli));
    assert_eq!(record.context.source_platform.as_deref(), Some("codex"));
}

#[test]
fn a_repeated_identity_fails_the_later_proof() {
    let mut same_item = claim(NATIVE, 2, 1);
    same_item["item_id"] = json!(item(1));
    let mut same_row = claim(NATIVE, 2, 0);
    same_row["item_id"] = json!(item(2));
    let mut same_record = claim(NATIVE, 1, 1);
    same_record["item_id"] = json!(item(2));
    for second in [
        with(user(2, SKILL_BODY), same_item),
        with(user(2, SKILL_BODY), same_row),
        with(user(1, SKILL_BODY), same_record),
        // An unclaimed repeat of a claimed record disputes that claim.
        user(1, SKILL_BODY),
    ] {
        let lines = marked(&[with(user(1, SKILL_BODY), claim(NATIVE, 1, 0)), second]);
        assert_eq!(
            record_origins(&lines),
            [
                RecordOrigin::Claimed(Box::new(proof(1, 0))),
                RecordOrigin::Invalid(OriginInvalid::Duplicate),
            ]
        );
    }
    // A claim naming a record the session already emitted unclaimed.
    let lines = marked(&[
        user(1, SKILL_BODY),
        with(user(1, SKILL_BODY), claim(NATIVE, 1, 0)),
    ]);
    assert_eq!(
        record_origins(&lines),
        [
            RecordOrigin::Declined,
            RecordOrigin::Invalid(OriginInvalid::Duplicate),
        ]
    );
    // Two `origin_evidence` keys on one line are no closed claim.
    let record = user(1, SKILL_BODY).to_string();
    let twice = format!(
        "{{\"origin_evidence\":{0},\"origin_evidence\":{0},{1}",
        claim(NATIVE, 1, 0),
        &record[1..]
    );
    let lines = vec![header(NATIVE, Some(marker())), twice];
    assert_eq!(
        record_origins(&lines),
        [RecordOrigin::Invalid(OriginInvalid::Shape)]
    );
}

/// A key repeated anywhere inside the claim or marker fails that evidence
/// alone, even when its last value would pass. Raw lines, since a JSON object
/// builder would collapse the repeat before the parser sees it.
#[test]
fn a_repeated_key_inside_the_evidence_fails_it() {
    let raw_claim = |from: &str, to: &str| {
        let claim = claim(NATIVE, 1, 0).to_string();
        assert!(claim.contains(from), "{claim}");
        claim.replacen(from, to, 1)
    };
    let raw_record = |claim: &str| {
        let record = user(1, SKILL_BODY).to_string();
        format!("{{\"origin_evidence\":{claim},{}", &record[1..])
    };
    let other = uuid(9);
    for raw in [
        raw_claim("{", &format!("{{\"record_uuid\":\"{other}\",")),
        raw_claim("{", &format!("{{\"record\\u005fuuid\":\"{other}\",")),
        raw_claim("{", "{\"kind\":\"other\","),
        raw_claim("\"segment\":{", "\"segment\":{\"history\":\"paginated\","),
        raw_claim(
            "\"segment\":{",
            &format!("\"segment\":{{\"rollout_id\":\"{ROLLOUT}\","),
        ),
        raw_claim("\"row\":{", "\"row\":{\"index\":7,"),
        raw_claim("\"row\":{", "\"row\":{\"ordinal\":3,"),
    ] {
        let lines = vec![
            header(NATIVE, Some(marker())),
            raw_record(&raw),
            with(user(2, SKILL_BODY), claim(NATIVE, 2, 1)).to_string(),
        ];
        // The repeat fails its proof; the session goes on claiming.
        assert_eq!(
            record_origins(&lines),
            [
                RecordOrigin::Invalid(OriginInvalid::Shape),
                RecordOrigin::Claimed(Box::new(proof(2, 1))),
            ],
            "{raw}"
        );
    }

    // A repeated marker key opens the session as invalid; it still imports.
    let raw_header = |to: &str| {
        let header = header(NATIVE, Some(marker()));
        let from = "\"origin_evidence\":{";
        assert!(header.contains(from), "{header}");
        header.replacen(from, &format!("{from}{to}"), 1)
    };
    for header in [
        raw_header("\"version\":2,"),
        raw_header("\"contract\":\"memhub.codex.source_witness\","),
        raw_header(&format!("\"contract\":\"{CONTRACT}\",")),
    ] {
        let lines = vec![
            header.clone(),
            with(user(1, SKILL_BODY), claim(NATIVE, 1, 0)).to_string(),
            user(2, "$review").to_string(),
        ];
        assert_eq!(
            session_origin(&lines),
            (OriginMarker::Invalid, SessionOrigin::Invalid),
            "{header}"
        );
        assert_eq!(
            record_origins(&lines),
            [
                RecordOrigin::Invalid(OriginInvalid::SessionMarker),
                RecordOrigin::Invalid(OriginInvalid::SessionMarker),
            ],
            "{header}"
        );
        // An ordinary stream refuses it like any marker.
        let mut stream = StreamEvents::new(Host::Codex, SessionSource::ReadersCli);
        assert!(matches!(
            stream.push(&header).unwrap(),
            Some(StreamEvent::MalformedHeader {
                reason: HEADER_MISMATCH,
                ..
            })
        ));
    }
}

/// Identities are tracked per session: the next header starts afresh.
#[test]
fn each_header_starts_a_fresh_session() {
    let mut other = claim(OTHER, 1, 0);
    other["item_id"] = json!(item(1));
    let mut lines = marked(&[with(user(1, SKILL_BODY), claim(NATIVE, 1, 0))]);
    lines.push(header(OTHER, Some(marker())));
    lines.push(with(user(1, SKILL_BODY), other).to_string());
    lines.push(header(NATIVE, None));
    lines.push(with(user(1, SKILL_BODY), claim(NATIVE, 1, 0)).to_string());
    let origins: Vec<_> = evidence(&lines).into_iter().map(|(_, o)| o).collect();
    assert_eq!(
        origins,
        [
            Some(Origin::Session(SessionOrigin::Marked)),
            Some(Origin::Record(RecordOrigin::Claimed(Box::new(proof(1, 0))))),
            Some(Origin::Session(SessionOrigin::Marked)),
            Some(Origin::Record(RecordOrigin::Claimed(Box::new(
                InjectedContextProof {
                    native_session_id: OTHER.into(),
                    ..proof(1, 0)
                }
            )))),
            Some(Origin::Session(SessionOrigin::Withheld)),
            Some(Origin::Record(RecordOrigin::Invalid(
                OriginInvalid::Unmarked
            ))),
        ]
    );
}

/// A line that ends a session ends its evidence too; lines without a record
/// carry no sidecar.
#[test]
fn stream_boundaries_are_unchanged_in_evidence_mode() {
    let pr_link =
        json!({"type":"pr-link","prNumber":1,"prUrl":"https://x/pull/1","prRepository":"x/y"});
    let lines = vec![
        header(NATIVE, Some(marker())),
        with(
            json!({"type":"user","timestamp":TS,"message":{"content":SKILL_BODY}}),
            claim(NATIVE, 1, 0),
        )
        .to_string(),
        pr_link.to_string(),
        with(user(2, SKILL_BODY), claim(NATIVE, 2, 1)).to_string(),
        header(NATIVE, Some(marker())),
        with(user(2, SKILL_BODY), claim(NATIVE, 2, 1)).to_string(),
    ];
    let events = evidence(&lines);
    assert_eq!(events.len(), 5, "{events:?}");
    assert!(matches!(events[1], (StreamEvent::Dropped, None)));
    assert!(matches!(
        events[2],
        (StreamEvent::MalformedRecord { line: 3, .. }, None)
    ));
    assert!(matches!(
        events[3],
        (
            StreamEvent::Session(_),
            Some(Origin::Session(SessionOrigin::Marked))
        )
    ));
    assert_eq!(
        events[4].1,
        Some(Origin::Record(RecordOrigin::Claimed(Box::new(proof(2, 1)))))
    );
}

/// Evidence never changes a parsed record: a claimed line and its plain line
/// parse to the same record.
#[test]
fn evidence_leaves_the_parsed_record_unchanged() {
    let plain = user(2, SKILL_BODY);
    let claimed = parsed(evidence(&marked(&[with(
        plain.clone(),
        claim(NATIVE, 2, 5),
    )])));
    let unclaimed = parsed(evidence(&marked(std::slice::from_ref(&plain))));
    let mut ordinary = StreamEvents::new(Host::Codex, SessionSource::ReadersCli);
    let ordinary: Vec<_> = lines(header(NATIVE, None), &[plain])
        .iter()
        .filter_map(|line| ordinary.push_with_origin(line).unwrap())
        .collect();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed, unclaimed);
    assert_eq!(claimed, parsed(ordinary));
}

/// The ordinary stream keeps its exact contract: a header key it does not
/// know, the evidence marker included, fails the header; records are parsed as
/// before; and no sidecar is ever produced.
#[test]
fn the_ordinary_stream_is_unchanged() {
    for marker in [
        marker(),
        json!(null),
        json!({"contract":CONTRACT,"version":2}),
    ] {
        let mut stream = StreamEvents::new(Host::Codex, SessionSource::ReadersCli);
        assert!(matches!(
            stream.push(&header(NATIVE, Some(marker.clone()))).unwrap(),
            Some(StreamEvent::MalformedHeader {
                native_session_id: None,
                line: 1,
                reason: HEADER_MISMATCH,
            })
        ));
        assert!(
            stream
                .push(&user(1, "$review").to_string())
                .unwrap()
                .is_none()
        );
    }
    let mut unknown = serde_json::from_str::<Value>(&header(NATIVE, None)).unwrap();
    unknown["origin_evidence_v2"] = marker();
    let mut stream = StreamEvents::new(Host::Codex, SessionSource::ReadersCli);
    assert!(matches!(
        stream.push(&unknown.to_string()).unwrap(),
        Some(StreamEvent::MalformedHeader {
            native_session_id: None,
            reason: HEADER_MISMATCH,
            ..
        })
    ));
    // The same unknown key fails the header in evidence mode too.
    assert!(matches!(
        evidence(&[unknown.to_string()]).as_slice(),
        [(
            StreamEvent::MalformedHeader {
                native_session_id: None,
                reason: HEADER_MISMATCH,
                ..
            },
            None
        )]
    ));

    // An unmarked stream yields the very same events in both modes.
    let pr_link =
        json!({"type":"pr-link","prNumber":1,"prUrl":"https://x/pull/1","prRepository":"x/y"});
    let stream_lines = vec![
        header(NATIVE, None),
        user(1, "$review").to_string(),
        with(user(2, SKILL_BODY), claim(NATIVE, 2, 5)).to_string(),
        json!({"type":"user","message":{"content":"no uuid"}}).to_string(),
        assistant(3).to_string(),
        pr_link.to_string(),
        user(4, "skipped").to_string(),
        header(OTHER, None),
        user(5, "next").to_string(),
    ];
    let mut ordinary = StreamEvents::new(Host::Codex, SessionSource::ReadersCli);
    let ordinary: Vec<_> = stream_lines
        .iter()
        .filter_map(|line| ordinary.push_with_origin(line).unwrap())
        .collect();
    assert!(ordinary.iter().all(|(_, origin)| origin.is_none()));
    let mut plain = StreamEvents::with_codex_origin_evidence();
    let plain: Vec<_> = stream_lines
        .iter()
        .filter_map(|line| plain.push(line).unwrap())
        .collect();
    let events = |events: Vec<StreamEvent>| format!("{events:?}");
    assert_eq!(
        events(ordinary.into_iter().map(|(event, _)| event).collect()),
        events(plain)
    );
    assert_eq!(
        StreamEvents::new(Host::Codex, SessionSource::ReadersCli)
            .push(&user(1, "early").to_string())
            .unwrap_err(),
        StreamEvents::with_codex_origin_evidence()
            .push(&user(1, "early").to_string())
            .unwrap_err()
    );
}
