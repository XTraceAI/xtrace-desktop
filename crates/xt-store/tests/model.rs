use serde_json::json;
use xt_store::{CanonicalRecord, model::RecordType};

#[test]
fn canonical_record_parses_claude_line() {
    let line = r#"{"uuid":"synthetic-claude","type":"assistant","timestamp":"2026-09-01T12:00:00Z","cwd":"/synthetic/project","gitBranch":"feature-synthetic","isMeta":false,"isSidechain":true,"requestId":"synthetic-request","message":{"id":"synthetic-response","role":"assistant","model":"synthetic-model","content":[{"type":"text","text":"Synthetic answer"},{"type":"tool_use","id":"synthetic-tool","name":"Read","input":{"path":"example.txt"}}],"usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":4,"cache_creation":{"ephemeral_5m_input_tokens":1,"ephemeral_1h_input_tokens":3},"service_tier":"standard"}}}"#;
    let record: CanonicalRecord = serde_json::from_str(line).unwrap();
    assert_eq!(record.record_type, RecordType::Assistant);
    assert!(record.is_sidechain);
    assert_eq!(record.message.id.as_deref(), Some("synthetic-response"));
    assert_eq!(record.request_id.as_deref(), Some("synthetic-request"));
    assert_eq!(
        record
            .message
            .usage
            .as_ref()
            .unwrap()
            .cache_creation
            .as_ref()
            .unwrap()
            .ephemeral_1h_input_tokens,
        Some(3)
    );
    assert_eq!(
        serde_json::from_value::<CanonicalRecord>(serde_json::to_value(&record).unwrap()).unwrap(),
        record
    );
}

#[test]
fn canonical_record_parses_cursor_line_without_usage() {
    let record: CanonicalRecord = serde_json::from_value(json!({
        "uuid":"synthetic-cursor","type":"user","source_surface":"cursor-agent-cli",
        "native_session_id":"synthetic-native","message":{"role":"user","content":[]}
    }))
    .unwrap();
    assert_eq!(record.timestamp, None);
    assert_eq!(record.message.usage, None);
    assert!(!record.is_meta && !record.is_sidechain);
    assert_eq!(record.source_surface.as_deref(), Some("cursor-agent-cli"));
    assert_eq!(
        record.native_session_id.as_deref(),
        Some("synthetic-native")
    );
}

#[test]
fn aliases_and_missing_uuid_are_supported_without_inventing_fields() {
    let record: CanonicalRecord = serde_json::from_value(json!({
        "type":"assistant","api_message_id":"canonical-api","request_id":"canonical-request",
        "git_branch":"synthetic-branch","message":{}
    }))
    .unwrap();
    assert_eq!(record.uuid, None);
    assert_eq!(record.api_message_id.as_deref(), Some("canonical-api"));
    assert_eq!(record.request_id.as_deref(), Some("canonical-request"));
    assert_eq!(record.message.content, None);
    assert!(
        serde_json::from_value::<CanonicalRecord>(
            json!({"uuid":"bad","type":"system","message":{}})
        )
        .is_err()
    );
}
