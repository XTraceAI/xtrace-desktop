//! Content-free measurement projection shared by ingestion and readers.
//! Field positions and encoding are fixed per version. Never include
//! transcript content, titles, tool input/output, or observation wall-clock time.
//!
//! Version 2 appends one field to version 1's 28: whether a structural
//! confirmation says another agent submitted this input. The field is present
//! only for a confirmed input, so an unconfirmed record has version 1's mask.
//! Incoming payloads can never state it — a canonical record has no such field —
//! so a confirmed record's current measurement differs from anything a capture
//! witnessed, and a receipt cannot verify the corrected classification.
//!
//! Version 1 digests remain readable, never reinterpreted: a version 1 revision
//! is recomputed with version 1's bytes, and exists only for a record that no
//! confirmation has changed.

use crate::{CanonicalRecord, Result, StoredRecord, model::RecordIdentity, timestamp, write};
use serde_json::{Value, json};

/// The version every newly computed revision uses.
pub const SCHEMA_VERSION: u32 = 2;
/// The legacy encoding: [`FIELDS`] without its final entry.
pub const LEGACY_SCHEMA_VERSION: u32 = 1;
const LEGACY_FIELD_COUNT: usize = 28;
/// Stable bit ordering. Unknown optional values have no bit; measured zero does.
pub const FIELDS: &[&str] = &[
    "uuid",
    "session",
    "type",
    "timestamp",
    "api_message_id",
    "request_id",
    "is_meta",
    "is_sidechain",
    "role",
    "model",
    "tool_result_carrier",
    "text_len",
    "tool_use_count",
    "tools",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_creation_tokens",
    "cache_creation_5m",
    "cache_creation_1h",
    "service_tier",
    "parent_uuid",
    "agent_id",
    "subtype",
    "is_human",
    "is_command",
    "is_interrupted",
    "is_system_reminder",
    "confirmed_automated_input",
];
const _: () = assert!(FIELDS.len() == LEGACY_FIELD_COUNT + 1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    fields: Vec<Option<Value>>,
}

impl Projection {
    pub fn from_canonical(
        session: &str,
        input: &CanonicalRecord,
        identity: &RecordIdentity,
    ) -> Result<Self> {
        let uuid = input
            .uuid
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .ok_or(crate::Error::InvalidInput("measurement requires a UUID"))?;
        let mut row = write::prepare(input, uuid, session, false)?;
        row.identity = identity.clone();
        Self::from_stored(&row)
    }

    pub fn from_stored(row: &StoredRecord) -> Result<Self> {
        let time = row
            .ts
            .as_deref()
            .map(timestamp::parse)
            .transpose()?
            .map(|(key, _)| key.components());
        let usage = row.usage.as_ref();
        let cache = usage.and_then(|usage| usage.cache_creation.as_ref());
        let tools = row.tool_use_count.map(|_| {
            row.tool_uses
                .iter()
                .map(|tool| json!([tool.block_index, tool.name]))
                .collect::<Vec<_>>()
        });
        let fields = vec![
            json!(row.uuid),
            json!(row.session_id),
            json!(row.record_type),
            json!(time),
            json!(row.api_message_id),
            json!(row.request_id),
            json!(row.is_meta),
            json!(row.is_sidechain),
            json!(row.role),
            json!(row.model),
            json!(row.is_tool_result_carrier),
            json!(row.text_len),
            json!(row.tool_use_count),
            json!(tools),
            json!(usage.and_then(|u| u.input_tokens)),
            json!(usage.and_then(|u| u.output_tokens)),
            json!(usage.and_then(|u| u.cache_read_input_tokens)),
            json!(usage.and_then(|u| u.cache_creation_input_tokens)),
            json!(cache.and_then(|c| c.ephemeral_5m_input_tokens)),
            json!(cache.and_then(|c| c.ephemeral_1h_input_tokens)),
            json!(usage.and_then(|u| u.service_tier.as_ref())),
            json!(row.identity.parent_uuid),
            json!(row.identity.agent_id),
            json!(row.identity.subtype),
            json!(row.classification.is_human),
            json!(row.classification.is_command),
            json!(row.classification.is_interrupted),
            json!(row.classification.is_system_reminder),
            json!(row.confirmed_automated_input.then_some(true)),
        ]
        .into_iter()
        .map(|value| (!value.is_null()).then_some(value))
        .collect();
        Ok(Self { fields })
    }

    /// Union measurements supplied by repeated UUIDs in one payload only.
    /// Contradictory values cannot define one immutable receipt revision.
    pub fn merge_known(&mut self, incoming: &Self) -> Result<()> {
        if self.conflicting_fields(incoming) != 0 {
            return Err(crate::Error::InvalidInput(
                "repeated submitted UUID has conflicting measurements",
            ));
        }
        for (saved, new) in self.fields.iter_mut().zip(&incoming.fields) {
            if saved.is_none() {
                *saved = new.clone();
            }
        }
        Ok(())
    }

    pub fn field_mask(&self) -> i64 {
        self.fields
            .iter()
            .enumerate()
            .fold(0, |mask, (bit, value)| {
                mask | if value.is_some() { 1_i64 << bit } else { 0 }
            })
    }

    /// Version and fixed-order values are hashed together; null stays unknown.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&(SCHEMA_VERSION, &self.fields)).expect("JSON values are serializable")
    }

    /// The mask and hashed bytes this measurement has under one digest schema
    /// version, or `None` when that version cannot describe it: an unknown
    /// version, or version 1 for a record a confirmation has changed, whose
    /// current classification version 1 has no position for.
    pub fn encoded(&self, version: u32) -> Option<(i64, Vec<u8>)> {
        match version {
            SCHEMA_VERSION => Some((self.field_mask(), self.canonical_bytes())),
            LEGACY_SCHEMA_VERSION => {
                let (legacy, added) = self.fields.split_at(LEGACY_FIELD_COUNT);
                if added.iter().any(Option::is_some) {
                    return None;
                }
                let mask = self.field_mask();
                let bytes = serde_json::to_vec(&(LEGACY_SCHEMA_VERSION, legacy))
                    .expect("JSON values are serializable");
                Some((mask, bytes))
            }
            _ => None,
        }
    }

    pub fn conflicting_fields(&self, incoming: &Self) -> i64 {
        self.fields
            .iter()
            .zip(&incoming.fields)
            .enumerate()
            .fold(0, |mask, (bit, (old, new))| {
                mask | if old
                    .as_ref()
                    .zip(new.as_ref())
                    .is_some_and(|(old, new)| old != new)
                {
                    1_i64 << bit
                } else {
                    0
                }
            })
    }
}
