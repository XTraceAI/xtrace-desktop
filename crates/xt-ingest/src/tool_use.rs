//! Structural events that are not assistant tool calls: slash commands, which
//! a user record states, and hook summaries, which are their own native record.
//!
//! Neither is a tool call, so neither may enter a record's `tool_use_count` or
//! its tool-call rows. Both are bound to the canonical session by the UUID the
//! native record already carries; nothing here invents an identity, reads a
//! command line, an argument list or any other free text.
//!
//! The name → kind mapping itself lives in [`xt_store::tool_use`], which the
//! canonical writer also uses so classification happens before content discard.

use crate::canonical::{ParsedRecord, StopHookSummary};
use serde_json::Value;
use xt_store::{
    SessionSource,
    ingest::{ToolEvent, ToolKind},
    model::RecordType,
};

pub use xt_store::tool_use::{
    HOOK_EVENT_NAME, MCP_PREFIX, SKILL_TOOL, SUBAGENT_TOOLS, Structure, classify,
};

const COMMAND_OPEN: &str = "<command-name>";
const COMMAND_CLOSE: &str = "</command-name>";

/// The slash command a user record states, if it states one. The record is
/// still stored as an ordinary record; this is the separate structural row.
/// A record whose command name is absent or blank yields no event rather than
/// a fabricated one.
pub fn command_event(
    record: &ParsedRecord,
    session_id: &str,
    source: SessionSource,
) -> Option<ToolEvent> {
    // Only a user record states a slash command. An assistant record quoting
    // the marker, and a record whose role is missing, are lookalikes: the
    // marker alone is not evidence that the user invoked anything, and an
    // unstated role is unknown rather than assumed to be the user's.
    if record.canonical.record_type != RecordType::User
        || record.canonical.message.role.as_deref() != Some("user")
    {
        return None;
    }
    let uuid = stable_id(record.canonical.uuid.as_deref())?;
    let name = command_name(&record.canonical)?;
    Some(ToolEvent {
        session_id: session_id.to_owned(),
        source,
        source_event_id: uuid,
        timestamp: record.canonical.timestamp.clone(),
        name,
        kind: ToolKind::Command,
        server: None,
        tool: None,
        skill: None,
    })
}

/// One event per stop-hook summary record, whatever `hookCount` it reports:
/// M-17 counts summaries, not the invocations one summary describes. Returns
/// `None` when the summary carries no stable UUID; such a record is unsupported
/// and is never given an arrival-ordered identity instead.
pub fn hook_event(
    summary: &StopHookSummary,
    session_id: &str,
    source: SessionSource,
) -> Option<ToolEvent> {
    Some(ToolEvent {
        session_id: session_id.to_owned(),
        source,
        source_event_id: stable_id(summary.uuid.as_deref())?,
        timestamp: summary.timestamp.clone(),
        name: HOOK_EVENT_NAME.to_owned(),
        kind: ToolKind::Hook,
        server: None,
        tool: None,
        skill: None,
    })
}

fn stable_id(uuid: Option<&str>) -> Option<String> {
    uuid.map(str::trim)
        .filter(|uuid| !uuid.is_empty())
        .map(str::to_owned)
}

/// Read the command name out of the structural marker the host writes, and
/// nothing else: the arguments, the local stdout and the rest of the message
/// are not parsed and never leave this function.
fn command_name(record: &xt_store::CanonicalRecord) -> Option<String> {
    let text = record
        .message
        .content
        .as_ref()?
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<String>();
    let rest = text.trim_start().strip_prefix(COMMAND_OPEN)?;
    let name = rest.split_once(COMMAND_CLOSE)?.0.trim();
    (!name.is_empty()).then(|| name.to_owned())
}
