//! Human-only assumptions must preserve supported missing data and every
//! agent-side measurement. All session identities and records are synthetic.
use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
mod activity_support;
use xt_fixtures::TempDb;
use xt_metrics::{MetricsDb, PriceCatalog, TypingRate, Window};
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    creation::{CreationDisposition, CreationEvidence, CreationWitness, SessionCreationProof},
    human_input::{OriginManifest, SessionOrigin},
};

const NATIVE: &str = "00000000-0000-7000-8000-000000000001";
const SESSION: &str = "codex-00000000-0000-7000-8000-000000000001";
const PARENT: &str = "00000000-0000-7000-8000-000000000002";
fn window() -> Window {
    let start = "2026-09-01T00:00:00Z"
        .parse::<Timestamp>()
        .unwrap()
        .as_millisecond();
    Window::new(start, start + 120_000).unwrap()
}
fn database(message: Value) -> TempDb {
    let mut db = TempDb::empty().unwrap();
    let mut session = SessionMeta::new(SESSION, "codex", SessionSource::ReadersCli);
    session.native_session_id = Some(NATIVE.into());
    db.store_mut().upsert_session(&session, false).unwrap();
    let records:Vec<CanonicalRecord>=serde_json::from_value(json!([
        {"uuid":"input","type":"user","timestamp":"2026-09-01T00:00:00Z","message":message},
        {"uuid":"agent","type":"assistant","timestamp":"2026-09-01T00:01:00Z","message":{"role":"assistant","content":[{"type":"text","text":"reply"}]}}
    ])).unwrap();
    db.store_mut()
        .upsert_records(SESSION, &records, false)
        .unwrap();
    db
}
fn assume(store: &mut Store, relation: bool) {
    if relation {
        let result = store
            .record_session_creations(
                &[SessionCreationProof {
                    child_session_id: SESSION.into(),
                    child_host: Host::Codex,
                    child_native_session_id: NATIVE.into(),
                    parent_host: Host::Codex,
                    parent_native_session_id: PARENT.into(),
                    evidence_kind: CreationEvidence::CodexThreadSpawn,
                    evidence_version: 1,
                    witness: CreationWitness::RolloutOpeningSessionMeta,
                }],
                1,
            )
            .unwrap();
        assert_eq!(result.dispositions, [CreationDisposition::Recorded]);
    } else {
        let result = store
            .import_human_session_origins(&OriginManifest {
                version: 1,
                sessions: vec![SessionOrigin {
                    session_id: SESSION.into(),
                    host: Host::Codex,
                    native_session_id: NATIVE.into(),
                    parent_host: Host::Codex,
                    parent_native_session_id: PARENT.into(),
                    method: "explicit_session_id".into(),
                    evidence_id: "synthetic-evidence".into(),
                    launch_id: "synthetic-call".into(),
                }],
            })
            .unwrap();
        assert_eq!(result.applied, 1);
    }
}
#[test]
fn missing_role_stays_unknown_and_readable_after_origin_or_relation() {
    for relation in [false, true] {
        let mut db = database(json!({}));
        let before = db.store().records(SESSION).unwrap();
        assert_eq!(before[0].role, None);
        assert_eq!(before[0].human_is_eligible, None);
        assert!(!before[0].human_excluded);
        let metrics = MetricsDb::open(db.path()).unwrap();
        let before_time = metrics
            .human_time(window(), TypingRate::default(), TimeZone::UTC)
            .unwrap();
        assert_eq!(before_time.human_minutes_est, None);
        let shared = activity_support::matches_standalone(
            &metrics,
            window(),
            TimeZone::UTC,
            TypingRate::default(),
            false,
            &PriceCatalog::bundled().unwrap(),
        );
        // Unknown human input stays unknown beside the known agent time.
        assert_eq!(shared.human, before_time);
        assert_eq!(shared.spans.active_ms, 60_000);
        assume(db.store_mut(), relation);
        // A missing role cannot establish that this is a user input. The
        // exclusion flag is false, but classification/length stay unknown.
        assert_eq!(db.store().records(SESSION).unwrap(), before);
        assert_eq!(
            metrics
                .human_time(window(), TypingRate::default(), TimeZone::UTC)
                .unwrap(),
            before_time
        );
    }
}
#[test]
fn unknown_original_turn_classification_survives_human_exclusion() {
    for relation in [false, true] {
        let mut db = database(json!({"role":"user","content":[{"type":"text","text":"request"}]}));
        Connection::open(db.path())
            .unwrap()
            .execute("UPDATE records SET is_human=NULL WHERE uuid='input'", [])
            .unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let mut before = metrics.counts(window(), TypingRate::default()).unwrap();
        let spans = metrics.active_spans(window()).unwrap();
        assert_eq!(before.human_messages, None);
        assert_eq!(before.assistant_turns, None);
        assume(db.store_mut(), relation);
        let after = metrics.counts(window(), TypingRate::default()).unwrap();
        assert_eq!(after.human_messages, Some(0));
        assert_eq!(after.human_chars, Some(0));
        assert_eq!(after.assistant_turns, None);
        before.human_messages = Some(0);
        before.human_chars = Some(0);
        before.typing_minutes_est = Some(0.0);
        assert_eq!(after, before);
        assert_eq!(metrics.active_spans(window()).unwrap(), spans);
    }
}
#[test]
fn unknown_human_eligibility_cannot_poison_known_agent_turns() {
    let db = database(json!({"role":"user","content":[{"type":"text","text":"request"}]}));
    let metrics = MetricsDb::open(db.path()).unwrap();
    let before = metrics.counts(window(), TypingRate::default()).unwrap();
    assert_eq!(before.assistant_turns, Some(1));
    // Deliberately vary only the Human projection to prove that turn
    // unknownness does not depend on that separate classification.
    Connection::open(db.path()).unwrap().execute_batch("DROP VIEW v_session_events;
        CREATE VIEW v_session_events AS SELECT session_id,uuid,ts,ts_ms,role,is_human,text_len,human_text_len,tool_use_count,confirmed_automated_input,human_excluded,NULL AS human_is_eligible FROM v_records WHERE ts_ms IS NOT NULL").unwrap();
    let after = metrics.counts(window(), TypingRate::default()).unwrap();
    assert_eq!(after.human_messages, None);
    assert_eq!(after.human_chars, None);
    assert_eq!(after.assistant_turns, Some(1));
    assert_eq!(after.assistant_records, before.assistant_records);
    assert_eq!(after.tool_calls, before.tool_calls);
}
