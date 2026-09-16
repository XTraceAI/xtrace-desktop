//! Native record precedence, independent of arrival order and copied-prefix offsets.
use crate::{Result, Store, batch::NativeOrderSource};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;

fn precedes(connection: &Connection, before: &str, after: &str) -> Result<bool> {
    Ok(connection.query_row(
        "WITH RECURSIVE later(uuid) AS (
            SELECT after_uuid FROM native_response_order WHERE before_uuid=?1
            UNION SELECT e.after_uuid FROM native_response_order e JOIN later ON e.before_uuid=later.uuid
         ) SELECT EXISTS(SELECT 1 FROM later WHERE uuid=?2)",
        [before,after], |row|row.get(0),
    )?)
}

pub(crate) fn observe(
    connection: &Connection,
    source: &NativeOrderSource,
    records: &[crate::CanonicalRecord],
    accepted: &BTreeSet<&str>,
) -> Result<BTreeSet<String>> {
    if source.reset {
        connection.execute(
            "DELETE FROM native_response_heads WHERE source_key=?1",
            [&source.key],
        )?;
    }
    let mut changed = BTreeSet::new();
    for input in records {
        let Some(uuid) = input.uuid.as_deref().filter(|id| accepted.contains(id)) else {
            continue;
        };
        let record: Option<(String,String,String)> = connection.query_row(
            "SELECT r.api_message_id,r.request_id,r.session_id FROM records r JOIN sessions s ON s.session_id=r.session_id
             JOIN usage u ON u.uuid=r.uuid WHERE r.uuid=?1 AND s.host='claude' AND r.type='assistant' AND r.is_meta=0 AND r.has_conflict=0
             AND r.api_message_id IS NOT NULL AND r.request_id IS NOT NULL",[uuid],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?;
        let Some((api, request, owner)) = record else {
            continue;
        };
        if api.trim().is_empty() || request.trim().is_empty() {
            continue;
        }
        let previous:Option<String>=connection.query_row(
            "SELECT h.record_uuid FROM native_response_heads h JOIN records r ON r.uuid=h.record_uuid WHERE h.source_key=?1 AND h.api_message_id=?2 AND h.request_id=?3 AND r.has_conflict=0 AND r.is_meta=0",
            params![source.key,api,request],|row|row.get(0),
        ).optional()?;
        if let Some(previous) = previous {
            // A replay of an older immutable UUID is not a new snapshot and
            // must not reverse the proven order or move this file's head back.
            if previous == uuid || precedes(connection, uuid, &previous)? {
                continue;
            }
            if !precedes(connection, &previous, uuid)? {
                connection.execute(
                    "INSERT INTO native_response_order(before_uuid,after_uuid) VALUES(?1,?2)",
                    [previous.as_str(), uuid],
                )?;
                changed.insert(owner);
                changed.insert(connection.query_row(
                    "SELECT session_id FROM records WHERE uuid=?1",
                    [&previous],
                    |row| row.get(0),
                )?);
            }
        }
        connection.execute("INSERT INTO native_response_heads(source_key,api_message_id,request_id,record_uuid) VALUES(?1,?2,?3,?4)
            ON CONFLICT(source_key,api_message_id,request_id) DO UPDATE SET record_uuid=excluded.record_uuid",
            params![source.key,api,request,uuid])?;
    }
    Ok(changed)
}
impl Store {
    /// An empty/inert full read has no record batch to reset its continuation state.
    pub fn clear_native_order_source(&mut self, key: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM native_response_heads WHERE source_key=?1",
            [key],
        )?;
        Ok(())
    }
}
