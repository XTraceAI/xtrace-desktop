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
    let mut changed_responses = BTreeSet::new();
    for input in records {
        let Some(uuid) = input.uuid.as_deref().filter(|id| accepted.contains(id)) else {
            continue;
        };
        let record: Option<(String,String)> = connection.query_row(
            "SELECT r.api_message_id,r.request_id FROM records r JOIN sessions s ON s.session_id=r.session_id
             JOIN usage u ON u.uuid=r.uuid WHERE r.uuid=?1 AND s.host='claude' AND r.type='assistant' AND r.is_meta=0 AND r.has_conflict=0
             AND r.api_message_id IS NOT NULL AND r.request_id IS NOT NULL",[uuid],|row|Ok((row.get(0)?,row.get(1)?)),
        ).optional()?;
        let Some((api, request)) = record else {
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
                changed_responses.insert((api.clone(), request.clone()));
            }
        }
        connection.execute("INSERT INTO native_response_heads(source_key,api_message_id,request_id,record_uuid) VALUES(?1,?2,?3,?4)
            ON CONFLICT(source_key,api_message_id,request_id) DO UPDATE SET record_uuid=excluded.record_uuid",
            params![source.key,api,request,uuid])?;
    }
    let mut changed = BTreeSet::new();
    // A new bridge can change selection for contexts containing only earlier
    // or later snapshots, not either endpoint. Invalidate the response's full
    // context set once, including canonical owners and native copies.
    let mut contexts = connection.prepare(
        "SELECT DISTINCT m.session_id FROM records r
         JOIN sessions s ON s.session_id=r.session_id
         JOIN session_work_records m ON m.record_uuid=r.uuid
         WHERE s.host='claude' AND r.api_message_id=?1 AND r.request_id=?2",
    )?;
    for (api, request) in changed_responses {
        for session in contexts.query_map(params![api, request], |row| row.get::<_, String>(0))? {
            changed.insert(session?);
        }
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
