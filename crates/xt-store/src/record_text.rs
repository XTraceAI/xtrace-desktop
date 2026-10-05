//! The text a stored user record said, read only where content retention kept
//! it. Nothing here classifies anything: which record is a person's message is
//! the stored M-02 classification's answer, and this module only reads back
//! the words of a record a caller already chose.

use crate::Result;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

/// The record's `text` blocks joined in order, exactly as the human-input
/// classifier joins them before it decides whether a record is a person's
/// message. Tool results, images and every other block kind add nothing.
pub fn joined(blocks: &[Value]) -> String {
    blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect()
}

/// What one stored record's text is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordText {
    /// No record has this identifier.
    Missing,
    /// The record exists but its content was not kept: metadata-only
    /// retention, or a purge since. Its words are unknown, never empty.
    NotStored,
    /// The joined text blocks, untrimmed.
    Stored(String),
}

/// Read back the joined text of exactly one record by its UUID. A record
/// whose stored content is not a block array has no text this rule can join,
/// so it is reported as not stored rather than as an empty message.
pub fn text(connection: &Connection, uuid: &str) -> Result<RecordText> {
    let content: Option<Option<String>> = connection
        .query_row(
            "SELECT content_json FROM records WHERE uuid=?1",
            [uuid],
            |row| row.get(0),
        )
        .optional()?;
    Ok(match content {
        None => RecordText::Missing,
        Some(None) => RecordText::NotStored,
        Some(Some(json)) => match serde_json::from_str::<Value>(&json)? {
            Value::Array(blocks) => RecordText::Stored(joined(&blocks)),
            _ => RecordText::NotStored,
        },
    })
}
