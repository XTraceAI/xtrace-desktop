use super::support::*;
use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::TempDir;
use xt_store::{Host, SessionMeta, SessionSource, Store, SurfaceEvidence, WriteStats};

#[test]
fn metadata_retention_is_future_only() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("privacy.sqlite");
    let mut store = Store::open(&path).unwrap();
    let mut saved_meta = session("saved");
    saved_meta.title = Some("saved synthetic title".into());
    store.upsert_session(&saved_meta, true).unwrap();
    let original = rich_record("saved-record");
    store
        .upsert_records("saved", std::slice::from_ref(&original), true)
        .unwrap();
    let saved_record = store.records("saved").unwrap().remove(0);
    let mut incoming = original.clone();
    incoming.message.content.as_mut().unwrap()[0]["text"] = json!("new private draft");
    incoming.message.content.as_mut().unwrap()[1]["input"] = json!({"path":"new-private.txt"});
    saved_meta.title = Some("new private title".into());
    store.upsert_session(&saved_meta, false).unwrap();
    assert_eq!(
        store
            .upsert_records("saved", &[incoming], false)
            .unwrap()
            .inserted,
        0
    );
    let after = store.records("saved").unwrap().remove(0);
    assert_eq!(after.content_json, saved_record.content_json);
    assert_eq!(after.tool_uses, saved_record.tool_uses);
    assert_eq!(
        store
            .session("saved")
            .unwrap()
            .unwrap()
            .meta
            .title
            .as_deref(),
        Some("saved synthetic title")
    );

    let mut fresh_meta = session("metadata-only");
    fresh_meta.title = Some("must not persist".into());
    store.upsert_session(&fresh_meta, false).unwrap();
    let mut content_without_usage = rich_record("metadata-record");
    content_without_usage.message.usage = None;
    content_without_usage.message.model = None;
    assert_eq!(
        store
            .upsert_records("metadata-only", &[content_without_usage], false)
            .unwrap()
            .inserted,
        1
    );
    assert_eq!(
        store
            .upsert_records("metadata-only", &[rich_record("metadata-record")], false)
            .unwrap(),
        WriteStats {
            enriched: 1,
            ..WriteStats::default()
        }
    );
    let row = store.records("metadata-only").unwrap().remove(0);
    assert_eq!(row.content_json, None);
    assert!(row.tool_uses.iter().all(|tool| tool.input_json.is_none()));
    assert_eq!((row.text_len, row.tool_use_count), (Some(3), Some(1)));
    assert_eq!(row.usage.unwrap().output_tokens, Some(4));
    assert_eq!(
        store.session("metadata-only").unwrap().unwrap().meta.title,
        None
    );
    let reader = Connection::open(path).unwrap();
    for sql in [
        "SELECT count(*) FROM sessions WHERE session_id='metadata-only' AND title IS NOT NULL",
        "SELECT count(*) FROM records WHERE session_id='metadata-only' AND content_json IS NOT NULL",
        "SELECT count(*) FROM tool_uses WHERE uuid='metadata-record' AND input_json IS NOT NULL",
    ] {
        assert_eq!(
            reader.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap(),
            0
        );
    }
    let evidence: String = reader
        .query_row(
            "SELECT surface_evidence_json FROM sessions WHERE session_id='metadata-only'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&evidence).unwrap(),
        json!({"source":"adapter-next","version":"2.1"})
    );
    for table in ["sessions", "records", "usage", "tool_uses"] {
        let columns: Vec<String> = reader
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(!columns.iter().any(|name| {
            ["text_head", "raw_json", "excerpt", "output_json"].contains(&name.as_str())
        }));
    }
}

#[test]
fn tool_rows_and_unicode_counts_come_from_blocks_before_content_discard() {
    let mut store = memory_store();
    let mut input = rich_record("blocks");
    input.message.content.as_mut().unwrap().extend([
        json!({"type":"text","text":"ok"}),
        json!({"type":"tool_use","name":"Bash","input":{"command":"echo synthetic"}}),
        json!({"type":"tool_result","content":"synthetic output is not human text"}),
        json!({"type":"future_block","data":"retained only in content mode"}),
    ]);
    store
        .upsert_records(SESSION, std::slice::from_ref(&input), false)
        .unwrap();
    assert_eq!(
        store
            .upsert_records(SESSION, &[input], false)
            .unwrap()
            .ignored,
        1
    );
    let row = store.records(SESSION).unwrap().remove(0);
    assert_eq!(
        (row.text_len, row.tool_use_count, row.is_tool_result_carrier),
        (Some(5), Some(2), Some(true))
    );
    assert_eq!(
        row.tool_uses
            .iter()
            .map(|t| (t.block_index, t.name.as_str()))
            .collect::<Vec<_>>(),
        [(1, "Read"), (3, "Bash")]
    );
    assert!(row.tool_uses.iter().all(|t| t.input_json.is_none()));
}

#[test]
fn unknown_platform_surface_and_structural_evidence_round_trip() {
    let mut store = Store::open_in_memory().unwrap();
    let mut metadata = SessionMeta::new("future-native", "new-agent", SessionSource::ReadersCli);
    metadata.surface = Some("new:surface/preview".into());
    metadata.native_session_id = Some("native-raw".into());
    metadata.surface_evidence = Some(SurfaceEvidence {
        source: "future-adapter".into(),
        version: Some("vNext".into()),
    });
    assert_eq!(metadata.host, Host::Other);
    store.upsert_session(&metadata, false).unwrap();
    assert_eq!(
        store.session("future-native").unwrap().unwrap().meta,
        metadata
    );
    metadata.host = Host::Claude;
    assert!(store.upsert_session(&metadata, false).is_err());
    metadata.host = Host::Other;
    metadata.surface_evidence.as_mut().unwrap().source = "/synthetic/private/path".into();
    assert!(store.upsert_session(&metadata, false).is_err());
    assert!(
        serde_json::from_value::<SurfaceEvidence>(json!({"source":"reader","excerpt":"private"}))
            .is_err()
    );
}

#[test]
fn conflicting_content_cannot_acquire_a_tool_input_projection() {
    let mut store = memory_store();
    let mut original = rich_record("coherent-content");
    original.message.content.as_mut().unwrap()[1]
        .as_object_mut()
        .unwrap()
        .remove("input");
    store
        .upsert_records(SESSION, &[original.clone()], true)
        .unwrap();
    let before = store.records(SESSION).unwrap().remove(0);
    assert_eq!(before.tool_uses[0].input_json, None);
    let mut incoming = rich_record("coherent-content");
    incoming.message.content.as_mut().unwrap()[0]["text"] = json!("abc");
    assert_eq!(
        store.upsert_records(SESSION, &[incoming], true).unwrap(),
        WriteStats {
            ignored: 1,
            ..WriteStats::default()
        }
    );
    let after = store.records(SESSION).unwrap().remove(0);
    assert!(after.has_conflict);
    assert_eq!(after.content_json, before.content_json);
    assert_eq!(after.tool_uses, before.tool_uses);
    assert_eq!(after.text_len, before.text_len);
}

#[test]
fn opting_back_into_content_acquires_a_coherent_missing_observation() {
    let mut store = memory_store();
    let input = rich_record("content-opt-in");
    store
        .upsert_records(SESSION, std::slice::from_ref(&input), false)
        .unwrap();
    let metadata = store.records(SESSION).unwrap().remove(0);
    assert!(metadata.content_json.is_none() && metadata.tool_uses[0].input_json.is_none());
    assert_eq!(
        store
            .upsert_records(SESSION, std::slice::from_ref(&input), true)
            .unwrap()
            .enriched,
        1
    );
    let retained = store.records(SESSION).unwrap().remove(0);
    assert!(!retained.has_conflict);
    assert_eq!(
        retained.content_json,
        input.message.content.map(Value::Array)
    );
    assert_eq!(
        retained.tool_uses[0].input_json,
        Some(json!({"path":"synthetic.txt"}))
    );
    assert_eq!(retained.text_len, metadata.text_len);
    assert_eq!(retained.tool_use_count, metadata.tool_use_count);
}
