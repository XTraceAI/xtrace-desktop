use crate::{Result, SessionMeta, Store, Usage, model::CacheCreation, model::RecordType};
use rusqlite::{Connection, OptionalExtension, Row, types::Type};
use serde::de::DeserializeOwned;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSession {
    pub meta: SessionMeta,
    pub first_ts: Option<String>,
    pub last_ts: Option<String>,
    pub has_conflict: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredToolUse {
    pub id: i64,
    pub block_index: i64,
    pub name: String,
    pub input_json: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRecord {
    pub uuid: String,
    pub session_id: String,
    pub record_type: RecordType,
    pub ts: Option<String>,
    pub ts_ms: Option<i64>,
    pub api_message_id: Option<String>,
    pub request_id: Option<String>,
    pub is_meta: bool,
    pub is_sidechain: bool,
    pub role: Option<String>,
    pub model: Option<String>,
    pub is_tool_result_carrier: Option<bool>,
    pub text_len: Option<i64>,
    pub tool_use_count: Option<i64>,
    pub content_json: Option<Value>,
    pub usage: Option<Usage>,
    pub tool_uses: Vec<StoredToolUse>,
    pub has_conflict: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreCounts {
    pub sessions: u64,
    pub records: u64,
    pub usage_rows: u64,
}

impl Store {
    pub fn session(&self, id: &str) -> Result<Option<StoredSession>> {
        session(&self.connection, id)
    }

    /// Timestamp order, with unknown timestamps last and UUID as a stable tie.
    /// Related usage/tool rows are read from the same SQLite snapshot.
    pub fn records(&self, session_id: &str) -> Result<Vec<StoredRecord>> {
        let transaction = self.connection.unchecked_transaction()?;
        let uuids = transaction
            .prepare(
                "SELECT uuid FROM records WHERE session_id=?1 ORDER BY ts_ms IS NULL, ts_ms, uuid",
            )?
            .query_map([session_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let records = uuids
            .iter()
            .map(|uuid| record(&transaction, uuid)?.ok_or(crate::Error::IncompatibleSchema))
            .collect::<Result<_>>()?;
        transaction.commit()?;
        Ok(records)
    }

    pub fn counts(&self) -> Result<StoreCounts> {
        Ok(self.connection.query_row(
            "SELECT (SELECT count(*) FROM sessions), (SELECT count(*) FROM records),
                    (SELECT count(*) FROM usage)",
            [],
            |row| {
                Ok(StoreCounts {
                    sessions: row.get::<_, i64>(0)? as u64,
                    records: row.get::<_, i64>(1)? as u64,
                    usage_rows: row.get::<_, i64>(2)? as u64,
                })
            },
        )?)
    }
}

pub(crate) fn session(connection: &Connection, id: &str) -> Result<Option<StoredSession>> {
    Ok(connection
        .query_row(
            "SELECT session_id, host, source_platform, source, cwd, git_branch, title,
                surface, surface_evidence_json, native_session_id, started_at_ms,
                first_ts, last_ts, has_conflict FROM sessions WHERE session_id=?1",
            [id],
            |row| {
                Ok(StoredSession {
                    meta: SessionMeta {
                        session_id: row.get(0)?,
                        host: row.get(1)?,
                        source_platform: row.get(2)?,
                        source: row.get(3)?,
                        cwd: row.get(4)?,
                        git_branch: row.get(5)?,
                        title: row.get(6)?,
                        surface: row.get(7)?,
                        surface_evidence: json_column(row, 8)?,
                        native_session_id: row.get(9)?,
                        started_at_ms: row.get(10)?,
                    },
                    first_ts: row.get(11)?,
                    last_ts: row.get(12)?,
                    has_conflict: row.get(13)?,
                })
            },
        )
        .optional()?)
}

pub(crate) fn record(connection: &Connection, uuid: &str) -> Result<Option<StoredRecord>> {
    let Some(mut record) = connection
        .query_row(
            "SELECT uuid, session_id, type, ts, ts_ms, api_message_id, request_id,
                is_meta, is_sidechain, role, model, is_tool_result_carrier,
                text_len, tool_use_count, content_json, has_conflict FROM records WHERE uuid=?1",
            [uuid],
            |row| {
                Ok(StoredRecord {
                    uuid: row.get(0)?,
                    session_id: row.get(1)?,
                    record_type: row.get(2)?,
                    ts: row.get(3)?,
                    ts_ms: row.get(4)?,
                    api_message_id: row.get(5)?,
                    request_id: row.get(6)?,
                    is_meta: row.get(7)?,
                    is_sidechain: row.get(8)?,
                    role: row.get(9)?,
                    model: row.get(10)?,
                    is_tool_result_carrier: row.get(11)?,
                    text_len: row.get(12)?,
                    tool_use_count: row.get(13)?,
                    content_json: json_column(row, 14)?,
                    usage: None,
                    tool_uses: Vec::new(),
                    has_conflict: row.get(15)?,
                })
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    record.usage = connection
        .query_row(
            "SELECT input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                cache_creation_5m, cache_creation_1h, service_tier FROM usage WHERE uuid=?1",
            [uuid],
            |row| {
                let five: Option<i64> = row.get(4)?;
                let hour: Option<i64> = row.get(5)?;
                Ok(Usage {
                    input_tokens: row.get(0)?,
                    output_tokens: row.get(1)?,
                    cache_read_input_tokens: row.get(2)?,
                    cache_creation_input_tokens: row.get(3)?,
                    cache_creation: if five.is_some() || hour.is_some() {
                        Some(CacheCreation {
                            ephemeral_5m_input_tokens: five,
                            ephemeral_1h_input_tokens: hour,
                        })
                    } else {
                        None
                    },
                    service_tier: row.get(6)?,
                })
            },
        )
        .optional()?;
    record.tool_uses = connection.prepare(
        "SELECT id, block_index, name, input_json FROM tool_uses WHERE uuid=?1 ORDER BY block_index",
    )?.query_map([uuid], |row| Ok(StoredToolUse {
        id: row.get(0)?, block_index: row.get(1)?, name: row.get(2)?, input_json: json_column(row, 3)?,
    }))?.collect::<rusqlite::Result<_>>()?;
    Ok(Some(record))
}

fn json_column<T: DeserializeOwned>(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<String>>(index)?
        .map(|json| {
            serde_json::from_str(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
            })
        })
        .transpose()
}
