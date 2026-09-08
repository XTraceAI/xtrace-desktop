use crate::{
    Result, SessionMeta, Store, Usage, model::CacheCreation, model::RecordType, timestamp,
};
use rusqlite::{Connection, OptionalExtension, Row, types::Type};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::BTreeMap;

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
    /// Coarse POSIX millisecond projection for indexing, not precise ordering.
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

    /// Native timestamp order at full fractional precision, with unknown times
    /// last and UUID as a stable tie only when instants are equal.
    /// Related usage/tool rows are read from the same SQLite snapshot.
    pub fn records(&self, session_id: &str) -> Result<Vec<StoredRecord>> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut records = transaction
            .prepare(&format!(
                "SELECT {RECORD_FIELDS} FROM records r WHERE r.session_id=?1"
            ))?
            .query_map([session_id], record_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|record| (record.uuid.clone(), record))
            .collect::<BTreeMap<_, _>>();
        for entry in transaction.prepare(&format!(
            "SELECT u.uuid,{USAGE_FIELDS} FROM usage u JOIN records r ON r.uuid=u.uuid WHERE r.session_id=?1"
        ))?.query_map([session_id], |row| Ok((row.get::<_, String>(0)?, usage_from_row(row, 1)?)))? {
            let (uuid, usage) = entry?;
            records.get_mut(&uuid).ok_or(crate::Error::IncompatibleSchema)?.usage = Some(usage);
        }
        for entry in transaction
            .prepare(&format!(
                "SELECT t.uuid,{TOOL_FIELDS} FROM tool_uses t JOIN records r ON r.uuid=t.uuid
             WHERE r.session_id=?1 ORDER BY t.uuid,t.block_index"
            ))?
            .query_map([session_id], |row| {
                Ok((row.get::<_, String>(0)?, tool_from_row(row, 1)?))
            })?
        {
            let (uuid, tool) = entry?;
            records
                .get_mut(&uuid)
                .ok_or(crate::Error::IncompatibleSchema)?
                .tool_uses
                .push(tool);
        }
        transaction.commit()?;
        // Sort owned rows after releasing the snapshot. No child-row read occurs
        // after commit, and UUID only breaks ties between equal native instants.
        let mut keyed = records
            .into_values()
            .map(|record| {
                let key = record
                    .ts
                    .as_deref()
                    .map(|value| timestamp::parse(value).map(|(key, _)| key))
                    .transpose()?;
                Ok((key, record))
            })
            .collect::<Result<Vec<_>>>()?;
        keyed.sort_unstable_by(|a, b| {
            (a.0.is_none(), &a.0, &a.1.uuid).cmp(&(b.0.is_none(), &b.0, &b.1.uuid))
        });
        Ok(keyed.into_iter().map(|(_, record)| record).collect())
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

pub(crate) fn timestamp_range(
    connection: &Connection,
    session_id: &str,
) -> Result<(Option<String>, Option<String>)> {
    // Use the millisecond index to limit precise comparison to endpoint buckets.
    // The 999ms guard includes a leap second whose POSIX projection overlaps the
    // following UTC second, so the coarse projection cannot discard an endpoint.
    let timestamps = ordered_timestamps(
        connection,
        session_id,
        "SELECT uuid, ts FROM records WHERE session_id=?1 AND (
            ts_ms BETWEEN (SELECT min(ts_ms) FROM records WHERE session_id=?1)
                AND (SELECT min(ts_ms)+999 FROM records WHERE session_id=?1)
            OR ts_ms BETWEEN (SELECT max(ts_ms)-999 FROM records WHERE session_id=?1)
                AND (SELECT max(ts_ms) FROM records WHERE session_id=?1))",
    )?;
    Ok((
        timestamps.first().and_then(|(_, ts)| ts.clone()),
        timestamps.last().and_then(|(_, ts)| ts.clone()),
    ))
}

fn ordered_timestamps(
    connection: &Connection,
    session_id: &str,
    query: &str,
) -> Result<Vec<(String, Option<String>)>> {
    let rows = connection
        .prepare(query)?
        .query_map([session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut keyed = rows
        .into_iter()
        .map(|(uuid, ts)| {
            let key = ts
                .as_deref()
                .map(|value| timestamp::parse(value).map(|(key, _)| key))
                .transpose()?;
            Ok((key, uuid, ts))
        })
        .collect::<Result<Vec<_>>>()?;
    keyed.sort_unstable_by(|a, b| (a.0.is_none(), &a.0, &a.1).cmp(&(b.0.is_none(), &b.0, &b.1)));
    Ok(keyed.into_iter().map(|(_, uuid, ts)| (uuid, ts)).collect())
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

const RECORD_FIELDS: &str = "r.uuid,r.session_id,r.type,r.ts,r.ts_ms,r.api_message_id,r.request_id,
    r.is_meta,r.is_sidechain,r.role,r.model,r.is_tool_result_carrier,r.text_len,r.tool_use_count,r.content_json,r.has_conflict";
const USAGE_FIELDS: &str =
    "u.input_tokens,u.output_tokens,u.cache_read_tokens,u.cache_creation_tokens,
    u.cache_creation_5m,u.cache_creation_1h,u.service_tier";
const TOOL_FIELDS: &str = "t.id,t.block_index,t.name,t.input_json";

pub(crate) fn record(connection: &Connection, uuid: &str) -> Result<Option<StoredRecord>> {
    let Some(mut record) = connection
        .query_row(
            &format!("SELECT {RECORD_FIELDS} FROM records r WHERE r.uuid=?1"),
            [uuid],
            record_from_row,
        )
        .optional()?
    else {
        return Ok(None);
    };
    record.usage = connection
        .query_row(
            &format!("SELECT {USAGE_FIELDS} FROM usage u WHERE u.uuid=?1"),
            [uuid],
            |row| usage_from_row(row, 0),
        )
        .optional()?;
    record.tool_uses = connection
        .prepare(&format!(
            "SELECT {TOOL_FIELDS} FROM tool_uses t WHERE t.uuid=?1 ORDER BY t.block_index"
        ))?
        .query_map([uuid], |row| tool_from_row(row, 0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(record))
}

fn record_from_row(row: &Row<'_>) -> rusqlite::Result<StoredRecord> {
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
        has_conflict: row.get(15)?,
        usage: None,
        tool_uses: Vec::new(),
    })
}

fn usage_from_row(row: &Row<'_>, start: usize) -> rusqlite::Result<Usage> {
    let five: Option<i64> = row.get(start + 4)?;
    let hour: Option<i64> = row.get(start + 5)?;
    Ok(Usage {
        input_tokens: row.get(start)?,
        output_tokens: row.get(start + 1)?,
        cache_read_input_tokens: row.get(start + 2)?,
        cache_creation_input_tokens: row.get(start + 3)?,
        cache_creation: if five.is_some() || hour.is_some() {
            Some(CacheCreation {
                ephemeral_5m_input_tokens: five,
                ephemeral_1h_input_tokens: hour,
            })
        } else {
            None
        },
        service_tier: row.get(start + 6)?,
    })
}

fn tool_from_row(row: &Row<'_>, start: usize) -> rusqlite::Result<StoredToolUse> {
    Ok(StoredToolUse {
        id: row.get(start)?,
        block_index: row.get(start + 1)?,
        name: row.get(start + 2)?,
        input_json: json_column(row, start + 3)?,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CanonicalRecord, SessionSource};
    use rusqlite::trace::{TraceEvent, TraceEventCodes};
    use serde_json::json;
    use std::cell::Cell;

    thread_local! { static SELECTS: Cell<usize> = const { Cell::new(0) }; }

    #[test]
    fn large_session_reads_use_bounded_queries_and_keep_children_with_their_record() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .upsert_session(
                &SessionMeta::new("large", "claude", SessionSource::Fixture),
                true,
            )
            .unwrap();
        let records = (0..1000).map(|n| {
            let mut record = json!({"uuid":format!("record-{:04}", 999-n),"type":"assistant",
                "timestamp":format!("2026-09-01T12:00:00.{n:010}Z"),
                "message":{"role":"assistant","content":[
                    {"type":"tool_use","name":"Read","input":{"sequence":n}},
                    {"type":"tool_use","name":"Write","input":{"sequence":n}}
                ]}});
            if n % 3 == 0 {
                record["message"]["usage"] = json!({"input_tokens":n,"output_tokens":null,"cache_read_input_tokens":0,"cache_creation_input_tokens":null});
            }
            serde_json::from_value::<CanonicalRecord>(record).unwrap()
        }).collect::<Vec<_>>();
        store.upsert_records("large", &records, true).unwrap();
        store
            .upsert_session(
                &SessionMeta::new("other", "claude", SessionSource::Fixture),
                true,
            )
            .unwrap();
        let mut other = records[0].clone();
        other.uuid = Some("other-record".into());
        store.upsert_records("other", &[other], true).unwrap();
        SELECTS.set(0);
        store.connection.trace_v2(
            TraceEventCodes::SQLITE_TRACE_STMT,
            Some(|event| {
                if let TraceEvent::Stmt(statement, _) = event
                    && statement.sql().trim_start().starts_with("SELECT")
                {
                    SELECTS.set(SELECTS.get() + 1);
                }
            }),
        );
        let loaded = store.records("large").unwrap();
        store.connection.trace_v2(TraceEventCodes::empty(), None);
        let statements = SELECTS.get();
        assert!(
            (1..=3).contains(&statements),
            "record reads must use at most three SELECTs, observed {statements}"
        );
        assert_eq!(loaded.len(), records.len());
        for (n, row) in loaded.iter().enumerate() {
            assert_eq!(row.uuid, format!("record-{:04}", 999 - n));
            assert_eq!(row.tool_uses.len(), 2);
            assert_eq!(row.tool_uses[0].name, "Read");
            assert_eq!(row.tool_uses[1].name, "Write");
            assert_eq!(row.tool_uses[0].input_json, Some(json!({"sequence":n})));
            if n % 3 == 0 {
                let usage = row.usage.as_ref().unwrap();
                assert_eq!(usage.input_tokens, Some(n as i64));
                assert_eq!(usage.output_tokens, None);
                assert_eq!(usage.cache_read_input_tokens, Some(0));
            } else {
                assert!(row.usage.is_none());
            }
        }
    }
}
