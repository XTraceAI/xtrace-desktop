use serde_json::json;
use xt_store::{
    CanonicalRecord, Host, SessionMeta, Store,
    human_input::{InputAdjustment, OriginManifest, SessionOrigin},
};
fn setup() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().join("index.db")).unwrap();
    let session: SessionMeta=serde_json::from_value(json!({"session_id":"codex-child","host":"codex","source":"readers_cli","native_session_id":"child"})).unwrap();
    store.upsert_session(&session, false).unwrap();
    let input: CanonicalRecord=serde_json::from_value(json!({"uuid":"input","type":"user","timestamp":"2026-09-01T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":"hello pasted"}]}})).unwrap();
    store
        .upsert_records("codex-child", &[input], false)
        .unwrap();
    (dir, store)
}
fn origin() -> SessionOrigin {
    SessionOrigin {
        session_id: "codex-child".into(),
        host: Host::Codex,
        native_session_id: "child".into(),
        parent_host: Host::Codex,
        parent_native_session_id: "parent".into(),
        method: "explicit_session_id".into(),
        evidence_id: "audit-1".into(),
        launch_id: "call-1".into(),
    }
}
#[test]
fn origin_is_bound_idempotent_and_does_not_claim_exact_proof() {
    let (dir, mut store) = setup();
    rusqlite::Connection::open(dir.path().join("index.db"))
        .unwrap()
        .execute(
            "UPDATE sessions SET has_conflict=1 WHERE session_id='codex-child'",
            [],
        )
        .unwrap();
    let before = store.records("codex-child").unwrap().remove(0);
    let manifest = OriginManifest {
        version: 1,
        sessions: vec![origin()],
    };
    assert_eq!(
        store
            .import_human_session_origins(&manifest)
            .unwrap()
            .applied,
        1
    );
    assert_eq!(
        store
            .import_human_session_origins(&manifest)
            .unwrap()
            .unchanged,
        1
    );
    let after = store.records("codex-child").unwrap().remove(0);
    assert_eq!(after.human_is_eligible, Some(false));
    assert!(after.human_excluded);
    assert!(!after.confirmed_automated_input);
    assert_eq!(before.classification, after.classification);
    assert_eq!(before.text_len, after.text_len);
    assert_eq!(before.content_json, after.content_json);
    assert!(after.content_json.is_none());
    let mut wrong = origin();
    wrong.native_session_id = "other".into();
    assert!(
        store
            .import_human_session_origins(&OriginManifest {
                version: 1,
                sessions: vec![wrong]
            })
            .is_err()
    );
    let mut conflict = origin();
    conflict.parent_native_session_id = "another-parent".into();
    assert_eq!(
        store
            .import_human_session_origins(&OriginManifest {
                version: 1,
                sessions: vec![conflict]
            })
            .unwrap()
            .conflicted,
        1
    );
    assert_eq!(
        store.records("codex-child").unwrap()[0].human_is_eligible,
        Some(true)
    );
}
#[test]
fn adjustment_preserves_raw_length_and_conflict_withholds_it() {
    let (_dir, mut store) = setup();
    let mut input = InputAdjustment {
        record_uuid: "input".into(),
        session_id: "codex-child".into(),
        original_ts: "2026-09-01T00:00:00Z".into(),
        original_length: 12,
        retained_length: Some(5),
        reason: "question_reply".into(),
        native_item_id: None,
    };
    assert_eq!(
        store
            .apply_human_input_adjustments(&[input.clone()])
            .unwrap()
            .applied,
        1
    );
    assert_eq!(
        store
            .apply_human_input_adjustments(&[input.clone()])
            .unwrap()
            .unchanged,
        1
    );
    let row = &store.records("codex-child").unwrap()[0];
    assert_eq!(row.human_text_len, Some(5));
    assert_eq!(row.text_len, Some(12));
    assert_eq!(row.human_is_eligible, Some(true));
    input.original_length = 13;
    assert_eq!(
        store
            .apply_human_input_adjustments(&[input])
            .unwrap()
            .conflicted,
        1
    );
    assert_eq!(
        store.records("codex-child").unwrap()[0].human_text_len,
        Some(12)
    );
}
