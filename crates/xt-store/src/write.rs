use crate::{
    CanonicalRecord, Error, Host, Result, SessionMeta, Store, StoredRecord, StoredSession,
    StoredToolUse, SurfaceEvidence, Usage, WriteStats,
    batch::{IngestBatchOutcome, RecordDisposition, RecordOutcome},
    model::CacheCreation,
    read, timestamp,
};
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::Value;

#[derive(Default)]
struct Change {
    enriched: bool,
    conflict: bool,
}

impl Change {
    fn fill<T: Clone + PartialEq>(&mut self, old: &mut Option<T>, incoming: &Option<T>) {
        if let Some(value) = incoming {
            match old {
                Some(saved) => self.conflict |= saved != value,
                None => {
                    *old = Some(value.clone());
                    self.enriched = true;
                }
            }
        }
    }
}

impl Store {
    /// Fill missing session metadata. A differing source is only a compatibility
    /// summary; it never establishes capture coverage. Saved titles are retained
    /// when `keep_content` is false, but incoming titles are not acquired.
    pub fn upsert_session(&mut self, meta: &SessionMeta, keep_content: bool) -> Result<()> {
        validate_session(meta)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        upsert_session(&transaction, meta, keep_content)?;
        transaction.commit()?;
        Ok(())
    }

    /// Atomically apply a batch. Each input contributes to exactly one counter.
    /// Missing UUIDs are omitted; other invalid input rolls back the whole batch.
    /// A UUID never changes session or record type. Conflicting known fields stay
    /// unchanged and set a content-free flag; only missing-field fills enrich.
    pub fn upsert_records(
        &mut self,
        session_id: &str,
        records: &[CanonicalRecord],
        keep_content: bool,
    ) -> Result<WriteStats> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = upsert_records(&transaction, session_id, records, keep_content, &[])?;
        transaction.commit()?;
        Ok(result.stats)
    }
}

// The public entry points validate the session before acquiring the write lock.
pub(crate) fn upsert_session(
    connection: &Connection,
    meta: &SessionMeta,
    keep_content: bool,
) -> Result<bool> {
    let before = read::session(connection, &meta.session_id)?;
    let stored = match before.clone() {
        Some(mut stored) => {
            merge_session(&mut stored, meta, keep_content);
            stored
        }
        None => {
            let mut meta = meta.clone();
            if !keep_content {
                meta.title = None;
            }
            StoredSession {
                meta,
                first_ts: None,
                last_ts: None,
                has_conflict: false,
            }
        }
    };
    save_session(connection, &stored)?;
    Ok(before.as_ref() != Some(&stored))
}

pub(crate) fn upsert_records(
    connection: &Connection,
    session_id: &str,
    records: &[CanonicalRecord],
    keep_content: bool,
    identities: &[crate::model::RecordIdentity],
) -> Result<IngestBatchOutcome> {
    let mut session = read::session(connection, session_id)?.ok_or(Error::InvalidInput(
        "session must be created before writing records",
    ))?;
    let previous_session = session.clone();
    let mut existing_records = read::records_by_uuid(
        connection,
        records
            .iter()
            .filter_map(|record| record.uuid.as_deref().filter(|id| !id.trim().is_empty())),
    )?;
    let mut stats = WriteStats::default();
    let mut outcomes = Vec::with_capacity(records.len());
    let mut affected_owners = std::collections::BTreeMap::new();
    for (input_index, input) in records.iter().enumerate() {
        let Some(uuid) = input.uuid.as_deref().filter(|id| !id.trim().is_empty()) else {
            stats.dropped_no_uuid += 1;
            outcomes.push(RecordOutcome {
                input_index,
                uuid: None,
                disposition: RecordDisposition::DroppedMissingUuid,
                conflict_fields: 0,
                stored_has_conflict: None,
            });
            continue;
        };
        let mut incoming = prepare(input, uuid, session_id, keep_content)?;
        incoming.identity = identities.get(input_index).cloned().unwrap_or_default();
        let metadata = SessionMeta {
            session_id: session_id.to_owned(),
            host: input
                .source_platform
                .as_deref()
                .map(Host::from_platform)
                .unwrap_or(session.meta.host),
            source_platform: input.source_platform.clone(),
            source: session.meta.source,
            cwd: input.cwd.clone(),
            git_branch: input.git_branch.clone(),
            title: None,
            surface: input.source_surface.clone(),
            surface_evidence: input.surface_evidence.clone(),
            native_session_id: input.native_session_id.clone(),
            started_at_ms: None,
        };
        validate_session(&metadata)?;
        // Ownership/type conflicts do not exempt nonblank input from validation.
        let mut existing = existing_records.get_mut(uuid);
        let conflict_fields = existing
            .as_ref()
            .map(|saved| {
                crate::measurement::Projection::from_stored(saved).and_then(|old| {
                    Ok(
                        old.conflicting_fields(&crate::measurement::Projection::from_stored(
                            &incoming,
                        )?),
                    )
                })
            })
            .transpose()?
            .unwrap_or(0);
        if let Some(stored) = existing.as_mut()
            && (stored.session_id != session_id || stored.record_type != incoming.record_type)
        {
            stored.has_conflict = true;
            if stored.session_id != session_id && !affected_owners.contains_key(&stored.session_id)
            {
                let owner = read::session(connection, &stored.session_id)?
                    .ok_or(Error::IncompatibleSchema)?;
                affected_owners.insert(
                    stored.session_id.clone(),
                    crate::batch::AffectedSession {
                        session_id: stored.session_id.clone(),
                        surface: owner.meta.surface,
                    },
                );
            }
            connection.execute("UPDATE records SET has_conflict=1 WHERE uuid=?1", [uuid])?;
            stats.ignored += 1;
            outcomes.push(RecordOutcome {
                input_index,
                uuid: Some(uuid.to_owned()),
                disposition: if stored.session_id != session_id {
                    RecordDisposition::RejectedOwnership
                } else {
                    RecordDisposition::RejectedType
                },
                stored_has_conflict: Some(true),
                conflict_fields,
            });
            continue;
        }
        let session_enriched = merge_session(&mut session, &metadata, false);
        let (disposition, stored_has_conflict) = match existing {
            None => {
                save_record(connection, &incoming)?;
                let conflict = incoming.has_conflict;
                existing_records.insert(uuid.to_owned(), incoming);
                stats.inserted += 1;
                (RecordDisposition::Inserted, conflict)
            }
            Some(stored) => {
                let before = stored.clone();
                let enriched = merge_record(stored, &incoming) || session_enriched;
                if before != *stored {
                    save_record(connection, stored)?;
                }
                let disposition = if enriched {
                    stats.enriched += 1;
                    RecordDisposition::Enriched
                } else {
                    stats.ignored += 1;
                    RecordDisposition::Duplicate
                };
                (disposition, stored.has_conflict)
            }
        };
        outcomes.push(RecordOutcome {
            input_index,
            uuid: Some(uuid.to_owned()),
            disposition,
            conflict_fields,
            stored_has_conflict: Some(stored_has_conflict),
        });
    }
    // Compare native instants, not lexically different RFC3339 offsets.
    // A native started_at_ms is never inferred from this imported range.
    (session.first_ts, session.last_ts) = read::timestamp_range(connection, session_id)?;
    save_session(connection, &session)?;
    connection.execute(
        "UPDATE sessions SET record_count=record_count+?1 WHERE session_id=?2",
        params![stats.inserted as i64, session_id],
    )?;
    Ok(IngestBatchOutcome {
        affected_owners: affected_owners.into_values().collect(),
        receipt_committed: false,
        session_changed: session != previous_session,
        stats,
        records: outcomes,
    })
}

pub(crate) fn validate_session(meta: &SessionMeta) -> Result<()> {
    if meta.session_id.trim().is_empty() {
        return Err(Error::InvalidInput("session ID is empty"));
    }
    if let Some(platform) = &meta.source_platform
        && (platform.trim().is_empty() || meta.host != Host::from_platform(platform))
    {
        return Err(Error::InvalidInput("raw platform and mapped host disagree"));
    }
    validate_evidence(meta.surface_evidence.as_ref())
}

fn validate_evidence(evidence: Option<&SurfaceEvidence>) -> Result<()> {
    if let Some(evidence) = evidence {
        for label in std::iter::once(&evidence.source).chain(evidence.version.iter()) {
            if label.is_empty()
                || label.len() > 128
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            {
                return Err(Error::InvalidInput(
                    "surface evidence requires structural labels",
                ));
            }
        }
    }
    Ok(())
}

fn merge_session(stored: &mut StoredSession, incoming: &SessionMeta, keep_content: bool) -> bool {
    let old = &mut stored.meta;
    let mut change = Change::default();
    if incoming.source_platform.is_some() {
        if old.source_platform.is_none() && (old.host == Host::Other || old.host == incoming.host) {
            old.host = incoming.host;
            change.fill(&mut old.source_platform, &incoming.source_platform);
        } else {
            change.conflict |= old.host != incoming.host;
            if old.host == incoming.host {
                change.fill(&mut old.source_platform, &incoming.source_platform);
            }
        }
    } else {
        change.conflict |= old.host != incoming.host;
    }
    change.fill(&mut old.cwd, &incoming.cwd);
    change.fill(&mut old.git_branch, &incoming.git_branch);
    if keep_content {
        change.fill(&mut old.title, &incoming.title);
    }
    change.fill(&mut old.surface, &incoming.surface);
    change.fill(&mut old.surface_evidence, &incoming.surface_evidence);
    change.fill(&mut old.native_session_id, &incoming.native_session_id);
    change.fill(&mut old.started_at_ms, &incoming.started_at_ms);
    stored.has_conflict |= change.conflict;
    change.enriched
}

pub(crate) fn prepare(
    input: &CanonicalRecord,
    uuid: &str,
    session_id: &str,
    keep_content: bool,
) -> Result<StoredRecord> {
    let ts_ms = input
        .timestamp
        .as_ref()
        .map(|text| timestamp::parse(text).map(|(_, milliseconds)| milliseconds))
        .transpose()?;
    if let Some(usage) = &input.message.usage {
        let cache = usage.cache_creation.as_ref();
        for counter in [
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_input_tokens,
            usage.cache_creation_input_tokens,
            cache.and_then(|c| c.ephemeral_5m_input_tokens),
            cache.and_then(|c| c.ephemeral_1h_input_tokens),
        ]
        .into_iter()
        .flatten()
        {
            if counter < 0 {
                return Err(Error::InvalidInput("usage counters must be nonnegative"));
            }
        }
    }
    let mut tools = Vec::new();
    let mut text_len = None;
    let mut carrier = None;
    if let Some(blocks) = &input.message.content {
        let mut characters = 0usize;
        let mut has_tool_result = false;
        for (index, block) in blocks.iter().enumerate() {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let text = block
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or(Error::InvalidInput("text block requires text"))?;
                    characters = characters
                        .checked_add(text.chars().count())
                        .ok_or(Error::InvalidInput("text length overflow"))?;
                }
                Some("tool_result") => has_tool_result = true,
                Some("tool_use") => {
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.trim().is_empty())
                        .ok_or(Error::InvalidInput("tool use requires a name"))?;
                    tools.push(StoredToolUse {
                        id: 0,
                        block_index: index as i64,
                        name: name.to_owned(),
                        input_json: if keep_content {
                            block.get("input").cloned()
                        } else {
                            None
                        },
                    });
                }
                Some(_) => {}
                None => return Err(Error::InvalidInput("content block requires a type")),
            }
        }
        text_len = Some(
            i64::try_from(characters).map_err(|_| Error::InvalidInput("text length overflow"))?,
        );
        carrier = Some(has_tool_result);
    }
    let classification = classify(input);
    Ok(StoredRecord {
        identity: crate::model::RecordIdentity::default(),
        classification,
        uuid: uuid.to_owned(),
        session_id: session_id.to_owned(),
        record_type: input.record_type,
        ts: input.timestamp.clone(),
        ts_ms,
        api_message_id: input
            .api_message_id
            .clone()
            .or_else(|| input.message.id.clone()),
        request_id: input.request_id.clone(),
        is_meta: input.is_meta,
        is_sidechain: input.is_sidechain,
        role: input.message.role.clone(),
        model: input.message.model.clone(),
        is_tool_result_carrier: carrier,
        text_len,
        tool_use_count: input.message.content.as_ref().map(|_| tools.len() as i64),
        content_json: if keep_content {
            input.message.content.clone().map(Value::Array)
        } else {
            None
        },
        usage: input.message.usage.clone(),
        tool_uses: tools,
        has_conflict: input
            .api_message_id
            .as_ref()
            .zip(input.message.id.as_ref())
            .is_some_and(|(a, b)| a != b),
    })
}

fn merge_record(old: &mut StoredRecord, incoming: &StoredRecord) -> bool {
    let mut change = Change {
        conflict: old.is_meta != incoming.is_meta || old.is_sidechain != incoming.is_sidechain,
        ..Change::default()
    };
    // Preserve the original spelling, but compare native instants so equivalent
    // UTC offsets are not conflicts. Sub-millisecond differences still conflict.
    match (&old.ts, &incoming.ts) {
        (None, Some(_)) => {
            old.ts = incoming.ts.clone();
            old.ts_ms = incoming.ts_ms;
            change.enriched = true;
        }
        (Some(saved), Some(new)) if saved != new => {
            change.conflict |= timestamp::parse(saved).ok().map(|(key, _)| key)
                != timestamp::parse(new).ok().map(|(key, _)| key);
        }
        _ => {}
    }
    change.fill(
        &mut old.identity.parent_uuid,
        &incoming.identity.parent_uuid,
    );
    change.fill(&mut old.identity.agent_id, &incoming.identity.agent_id);
    change.fill(&mut old.identity.subtype, &incoming.identity.subtype);
    if old.identity.first_seen_at.is_none() && incoming.identity.first_seen_at.is_some() {
        old.identity.first_seen_at = incoming.identity.first_seen_at;
        change.enriched = true;
    }
    change.fill(&mut old.api_message_id, &incoming.api_message_id);
    change.fill(&mut old.request_id, &incoming.request_id);
    change.fill(&mut old.role, &incoming.role);
    change.fill(&mut old.model, &incoming.model);
    let content_change = merge_content(old, incoming);
    if !content_change.conflict {
        change.fill(
            &mut old.classification.is_human,
            &incoming.classification.is_human,
        );
        change.fill(
            &mut old.classification.is_command,
            &incoming.classification.is_command,
        );
        change.fill(
            &mut old.classification.is_interrupted,
            &incoming.classification.is_interrupted,
        );
        change.fill(
            &mut old.classification.is_system_reminder,
            &incoming.classification.is_system_reminder,
        );
    }
    if let Some(usage) = &incoming.usage {
        match &mut old.usage {
            None => {
                old.usage = Some(usage.clone());
                change.enriched = true;
            }
            Some(saved) => merge_usage(&mut change, saved, usage),
        }
    }
    old.has_conflict |= incoming.has_conflict || change.conflict || content_change.conflict;
    change.enriched || content_change.enriched
}

fn merge_content(old: &mut StoredRecord, incoming: &StoredRecord) -> Change {
    fn differs<T: PartialEq>(a: &Option<T>, b: &Option<T>) -> bool {
        a.as_ref().zip(b.as_ref()).is_some_and(|(a, b)| a != b)
    }
    let both_measured = old.text_len.is_some() && incoming.text_len.is_some();
    let tool_conflict = both_measured
        && (old.tool_uses.len() != incoming.tool_uses.len()
            || old.tool_uses.iter().zip(&incoming.tool_uses).any(|(a, b)| {
                a.name != b.name
                    || a.block_index != b.block_index
                    || differs(&a.input_json, &b.input_json)
            }));
    // Retained content and its tool-input projection are one observation. Check
    // every known part before acquiring any missing part from a conflicting array.
    if differs(&old.content_json, &incoming.content_json)
        || differs(&old.text_len, &incoming.text_len)
        || differs(&old.tool_use_count, &incoming.tool_use_count)
        || differs(
            &old.is_tool_result_carrier,
            &incoming.is_tool_result_carrier,
        )
        || tool_conflict
    {
        return Change {
            conflict: true,
            ..Change::default()
        };
    }
    let mut change = Change::default();
    let content_was_unknown = old.text_len.is_none();
    change.fill(&mut old.text_len, &incoming.text_len);
    change.fill(&mut old.tool_use_count, &incoming.tool_use_count);
    change.fill(
        &mut old.is_tool_result_carrier,
        &incoming.is_tool_result_carrier,
    );
    if incoming.text_len.is_some() {
        if content_was_unknown {
            old.tool_uses = incoming.tool_uses.clone();
        }
        for (old_tool, new_tool) in old.tool_uses.iter_mut().zip(&incoming.tool_uses) {
            change.fill(&mut old_tool.input_json, &new_tool.input_json);
        }
        change.fill(&mut old.content_json, &incoming.content_json);
    }
    change
}

fn merge_usage(change: &mut Change, old: &mut Usage, new: &Usage) {
    change.fill(&mut old.input_tokens, &new.input_tokens);
    change.fill(&mut old.output_tokens, &new.output_tokens);
    change.fill(
        &mut old.cache_read_input_tokens,
        &new.cache_read_input_tokens,
    );
    change.fill(
        &mut old.cache_creation_input_tokens,
        &new.cache_creation_input_tokens,
    );
    change.fill(&mut old.service_tier, &new.service_tier);
    if let Some(new) = &new.cache_creation {
        let old = old
            .cache_creation
            .get_or_insert_with(CacheCreation::default);
        change.fill(
            &mut old.ephemeral_5m_input_tokens,
            &new.ephemeral_5m_input_tokens,
        );
        change.fill(
            &mut old.ephemeral_1h_input_tokens,
            &new.ephemeral_1h_input_tokens,
        );
    }
}

fn save_session(connection: &Connection, session: &StoredSession) -> Result<()> {
    let s = &session.meta;
    let evidence = s
        .surface_evidence
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    connection.execute(
        "INSERT INTO sessions(session_id,host,source_platform,source,cwd,git_branch,title,surface,
             surface_evidence_json,native_session_id,started_at_ms,first_ts,last_ts,has_conflict)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
         ON CONFLICT(session_id) DO UPDATE SET host=excluded.host,source_platform=excluded.source_platform,
             cwd=excluded.cwd,git_branch=excluded.git_branch,title=excluded.title,surface=excluded.surface,
             surface_evidence_json=excluded.surface_evidence_json,native_session_id=excluded.native_session_id,
             started_at_ms=excluded.started_at_ms,first_ts=excluded.first_ts,last_ts=excluded.last_ts,
             has_conflict=excluded.has_conflict",
        params![s.session_id,s.host,s.source_platform,s.source,s.cwd,s.git_branch,s.title,s.surface,
            evidence,s.native_session_id,s.started_at_ms,session.first_ts,session.last_ts,session.has_conflict],
    )?;
    Ok(())
}

fn save_record(connection: &Connection, record: &StoredRecord) -> Result<()> {
    let r = record;
    let content = r
        .content_json
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    connection.execute(
        "INSERT INTO records(uuid,session_id,type,ts,ts_ms,api_message_id,request_id,is_meta,is_sidechain,
             role,model,is_tool_result_carrier,text_len,tool_use_count,content_json,has_conflict,parent_uuid,agent_id,subtype,first_seen_at,is_human,is_command,is_interrupted,is_system_reminder)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)
         ON CONFLICT(uuid) DO UPDATE SET ts=excluded.ts,ts_ms=excluded.ts_ms,api_message_id=excluded.api_message_id,
             request_id=excluded.request_id,role=excluded.role,model=excluded.model,
             is_tool_result_carrier=excluded.is_tool_result_carrier,text_len=excluded.text_len,
             tool_use_count=excluded.tool_use_count,content_json=excluded.content_json,has_conflict=excluded.has_conflict,
             parent_uuid=excluded.parent_uuid,agent_id=excluded.agent_id,subtype=excluded.subtype,first_seen_at=excluded.first_seen_at,
             is_human=excluded.is_human,is_command=excluded.is_command,is_interrupted=excluded.is_interrupted,is_system_reminder=excluded.is_system_reminder",
        params![r.uuid,r.session_id,r.record_type,r.ts,r.ts_ms,r.api_message_id,r.request_id,r.is_meta,r.is_sidechain,
            r.role,r.model,r.is_tool_result_carrier,r.text_len,r.tool_use_count,content,r.has_conflict,
            r.identity.parent_uuid,r.identity.agent_id,r.identity.subtype,r.identity.first_seen_at,
            r.classification.is_human,r.classification.is_command,r.classification.is_interrupted,r.classification.is_system_reminder],
    )?;
    if let Some(u) = &r.usage {
        let five = u
            .cache_creation
            .as_ref()
            .and_then(|c| c.ephemeral_5m_input_tokens);
        let hour = u
            .cache_creation
            .as_ref()
            .and_then(|c| c.ephemeral_1h_input_tokens);
        connection.execute(
            "INSERT INTO usage(uuid,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,
                 cache_creation_5m,cache_creation_1h,service_tier) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(uuid) DO UPDATE SET input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens,
                 cache_read_tokens=excluded.cache_read_tokens,cache_creation_tokens=excluded.cache_creation_tokens,
                 cache_creation_5m=excluded.cache_creation_5m,cache_creation_1h=excluded.cache_creation_1h,service_tier=excluded.service_tier",
            params![r.uuid,u.input_tokens,u.output_tokens,u.cache_read_input_tokens,u.cache_creation_input_tokens,five,hour,u.service_tier],
        )?;
    }
    for tool in &r.tool_uses {
        let input = tool
            .input_json
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        connection.execute(
            "INSERT INTO tool_uses(uuid,session_id,block_index,name,input_json) VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(uuid,block_index) DO UPDATE SET input_json=excluded.input_json",
            params![r.uuid, r.session_id, tool.block_index, tool.name, input],
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;

fn classify(input: &CanonicalRecord) -> crate::model::RecordClassification {
    let Some(blocks) = &input.message.content else {
        return Default::default();
    };
    let text = blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<String>();
    let command = text.starts_with("<command-name>") || text.starts_with("<local-command-stdout>");
    let interrupted = text.starts_with("[Request interrupted");
    let reminder = text.starts_with("<system-reminder>");
    crate::model::RecordClassification {
        // The role-sensitive human rule belongs to its classification consumer.
        // Keep only facts that would
        // be lost by content discard; record type does not establish role.
        is_human: None,
        is_command: Some(command),
        is_interrupted: Some(interrupted),
        is_system_reminder: Some(reminder),
    }
}
