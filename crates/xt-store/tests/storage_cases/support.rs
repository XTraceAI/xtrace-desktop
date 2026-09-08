use serde_json::json;
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, Store};

pub(super) fn record(id: &str) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":id,"type":"assistant","message":{"role":"assistant"}}))
        .unwrap()
}

pub(super) fn rich_record(id: &str) -> CanonicalRecord {
    serde_json::from_value(json!({
        "uuid":id,"type":"assistant","timestamp":"2026-09-01T12:00:00.123Z",
        "requestId":"request-synthetic","source_surface":"future-desktop.v2",
        "native_session_id":"native-synthetic",
        "surface_evidence":{"source":"adapter-next","version":"2.1"},
        "message":{"id":"api-synthetic","role":"assistant","model":"model-synthetic",
            "content":[{"type":"text","text":"hé🐱"},
                {"type":"tool_use","name":"Read","input":{"path":"synthetic.txt"}}],
            "usage":{"input_tokens":10,"output_tokens":4,"cache_read_input_tokens":0,
                "cache_creation_input_tokens":5,
                "cache_creation":{"ephemeral_5m_input_tokens":2,"ephemeral_1h_input_tokens":3},
                "service_tier":"standard"}}
    }))
    .unwrap()
}

pub(super) fn session(id: &str) -> SessionMeta {
    SessionMeta::new(id, "codex", SessionSource::Fixture)
}

pub(super) fn memory_store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store
        .upsert_session(&session("codex-native-synthetic"), true)
        .unwrap();
    store
}

pub(super) const SESSION: &str = "codex-native-synthetic";
