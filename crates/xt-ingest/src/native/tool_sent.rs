//! Opt-in evidence that the Codex or Cursor tool, or the pinned reader itself,
//! wrote a user record rather than a person.
//!
//! With `--automated-input-evidence` the pinned producer adds one closed
//! `automated_input` object to each record line it claims (contract
//! `xtrace.automated_input`, version 1; see `vendor/agent-plugins`'
//! `readers/automated_input.py`). The claim rests on a structural marker in the
//! native source, never on text, and names only its kind, the native session
//! and the record UUID. This module judges it against the line's own parsed
//! record and session header: a claim that is not exactly the closed version 1
//! shape, names another session or record, names a kind of another host, or
//! sits on anything but a user record is refused for the proof alone. The
//! record is never changed, dropped or delayed. Nothing here reads, keeps or
//! echoes text, a digest, a path or a title.

use serde::Deserialize;
use serde_json::Value;
use xt_store::{CanonicalRecord, Host, model::RecordType, tool_sent::ToolSentKind};

pub const CONTRACT: &str = "xtrace.automated_input";
pub const VERSION: u64 = 1;

/// What the evidence says of one record line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolSentEvidence {
    /// No `automated_input` key on the line.
    Absent,
    /// A validated claim bound to this very record and session.
    Claimed(ToolSentKind),
    /// The claim is refused; the record itself stands.
    Refused(ToolSentRefusal),
}

/// Fixed refusal codes. None echoes a value from the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolSentRefusal {
    /// Not one closed object: missing, unknown or repeated keys, or `null`.
    Shape,
    Contract,
    Version,
    /// Not a kind this build knows, or a kind of another host.
    Kind,
    /// The claim names another session than its header.
    SessionMismatch,
    /// The claim names another record than its line.
    RecordMismatch,
    /// The claimed record is not a user record.
    NotUserRecord,
}

impl ToolSentRefusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::Shape => "tool_sent_claim_shape",
            Self::Contract => "tool_sent_claim_contract",
            Self::Version => "tool_sent_claim_version",
            Self::Kind => "tool_sent_claim_kind",
            Self::SessionMismatch => "tool_sent_claim_session_mismatch",
            Self::RecordMismatch => "tool_sent_claim_record_mismatch",
            Self::NotUserRecord => "tool_sent_claim_not_user_record",
        }
    }
}

/// Judge the evidence on one record line whose canonical record parsed, in a
/// session of `host` whose header names `native_session_id`.
pub fn judge(
    line: &str,
    host: Host,
    native_session_id: &str,
    record: &CanonicalRecord,
) -> ToolSentEvidence {
    // Everything else on the line is skipped unread. Derived structs refuse a
    // repeated key, at the top level or inside the claim, so a repeat can
    // never pass the closed shape by keeping only its last value.
    #[derive(Deserialize)]
    struct Probe {
        #[serde(default)]
        automated_input: Option<Claim>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Claim {
        contract: String,
        version: Value,
        kind: String,
        native_session_id: String,
        record_uuid: String,
    }
    if !has_key(line) {
        return ToolSentEvidence::Absent;
    }
    // A malformed or repeated claim, and `null` (present, not absent), are
    // refused for the proof alone.
    let Ok(Probe {
        automated_input: Some(claim),
    }) = serde_json::from_str::<Probe>(line)
    else {
        return ToolSentEvidence::Refused(ToolSentRefusal::Shape);
    };
    let refused = ToolSentEvidence::Refused;
    if claim.contract != CONTRACT {
        return refused(ToolSentRefusal::Contract);
    }
    // The JSON integer 1: not `1.0`, `"1"` or `true`.
    if claim.version.as_u64() != Some(VERSION) || !claim.version.is_u64() {
        return refused(ToolSentRefusal::Version);
    }
    let Some(kind) = ToolSentKind::parse(&claim.kind).filter(|kind| kind.host() == host) else {
        return refused(ToolSentRefusal::Kind);
    };
    if claim.native_session_id != native_session_id {
        return refused(ToolSentRefusal::SessionMismatch);
    }
    if record.uuid.as_deref() != Some(claim.record_uuid.as_str()) {
        return refused(ToolSentRefusal::RecordMismatch);
    }
    if record.record_type != RecordType::User || record.message.role.as_deref() != Some("user") {
        return refused(ToolSentRefusal::NotUserRecord);
    }
    ToolSentEvidence::Claimed(kind)
}

/// Whether the line's top-level object holds the key at all.
fn has_key(line: &str) -> bool {
    serde_json::from_str::<serde_json::Map<String, Value>>(line)
        .is_ok_and(|object| object.contains_key("automated_input"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NATIVE: &str = "0a0a0a0a-1111-4222-8333-444444444444";

    fn record(kind: &str, role: &str) -> CanonicalRecord {
        serde_json::from_value(json!({"uuid":"record-1","type":kind,
            "message":{"role":role,"content":[{"type":"text","text":"synthetic"}]}}))
        .unwrap()
    }

    fn line(claim: Value) -> String {
        json!({"uuid":"record-1","type":"user","message":{"role":"user","content":"synthetic"},
            "automated_input":claim})
        .to_string()
    }

    fn claim() -> Value {
        json!({"contract":CONTRACT,"version":1,"kind":"codex_turn_aborted",
            "native_session_id":NATIVE,"record_uuid":"record-1"})
    }

    #[test]
    fn a_claim_binds_only_to_its_own_record_session_and_host() {
        let user = record("user", "user");
        let take = |value: Value, host| judge(&line(value), host, NATIVE, &user);
        assert_eq!(
            take(claim(), Host::Codex),
            ToolSentEvidence::Claimed(ToolSentKind::CodexTurnAborted)
        );
        assert_eq!(
            take(claim(), Host::Cursor),
            ToolSentEvidence::Refused(ToolSentRefusal::Kind)
        );
        let mut banner = claim();
        banner["kind"] = json!("cursor_import_banner");
        assert_eq!(
            take(banner, Host::Cursor),
            ToolSentEvidence::Claimed(ToolSentKind::CursorImportBanner)
        );
        for (field, value, refusal) in [
            (
                "contract",
                json!("memhub.codex.origin_evidence"),
                ToolSentRefusal::Contract,
            ),
            ("version", json!(2), ToolSentRefusal::Version),
            ("version", json!(1.0), ToolSentRefusal::Version),
            ("version", json!("1"), ToolSentRefusal::Version),
            (
                "kind",
                json!("claude_task_notification"),
                ToolSentRefusal::Kind,
            ),
            ("kind", json!(1), ToolSentRefusal::Shape),
            (
                "native_session_id",
                json!("other"),
                ToolSentRefusal::SessionMismatch,
            ),
            (
                "record_uuid",
                json!("record-2"),
                ToolSentRefusal::RecordMismatch,
            ),
            ("extra", json!(true), ToolSentRefusal::Shape),
        ] {
            let mut bad = claim();
            bad[field] = value;
            assert_eq!(
                take(bad, Host::Codex),
                ToolSentEvidence::Refused(refusal),
                "{field}"
            );
        }
        let mut missing = claim();
        missing.as_object_mut().unwrap().remove("record_uuid");
        assert_eq!(
            take(missing, Host::Codex),
            ToolSentEvidence::Refused(ToolSentRefusal::Shape)
        );
        assert_eq!(
            take(Value::Null, Host::Codex),
            ToolSentEvidence::Refused(ToolSentRefusal::Shape)
        );
        assert_eq!(
            judge(
                &line(claim()),
                Host::Codex,
                NATIVE,
                &record("assistant", "assistant")
            ),
            ToolSentEvidence::Refused(ToolSentRefusal::NotUserRecord)
        );
        let plain = json!({"uuid":"record-1","type":"user","message":{"role":"user"}}).to_string();
        assert_eq!(
            judge(&plain, Host::Codex, NATIVE, &user),
            ToolSentEvidence::Absent
        );
    }

    #[test]
    fn a_repeated_key_refuses_the_claim_not_the_line() {
        let user = record("user", "user");
        let repeated = format!(
            "{{\"uuid\":\"record-1\",\"type\":\"user\",\"automated_input\":{},\"automated_input\":{}}}",
            claim(),
            claim()
        );
        assert_eq!(
            judge(&repeated, Host::Codex, NATIVE, &user),
            ToolSentEvidence::Refused(ToolSentRefusal::Shape)
        );
    }
}
