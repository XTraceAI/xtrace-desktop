//! The shared read projections the store installs. `v_record_metadata`,
//! `v_human_inputs` and `v_session_events` are plain SQL files; `v_records`,
//! `v_usage_records` and `v_response_usage` are assembled from the rule macros
//! in `lib.rs`, so the work-record filter, the keyed-response test, its
//! whitespace set and the latest-snapshot selection each have one definition
//! that the views and the per-session reads ([`crate::session_model`]) share.
//! The internal `v_record_metadata` joins each stored record with its session
//! and stored input facts once and makes each Human decision there; the
//! public `v_human_inputs` and `v_records` read it. Installing changed view
//! text replaces the views only; no stored row changes.

pub const RECORD_METADATA: &str = include_str!("../views/record_metadata.sql");

pub const HUMAN_INPUTS: &str = include_str!("../views/human_inputs.sql");

pub const RECORDS: &str = concat!(
    r#"CREATE VIEW v_records AS
SELECT m.uuid,m.session_id,m.type,m.ts,m.ts_ms,m.api_message_id,m.request_id,
       m.is_sidechain,m.role,m.model,m.is_tool_result_carrier,m.text_len,
       m.tool_use_count,
       m.human_is_eligible,m.human_text_len,m.human_excluded,
       m.is_human,m.raw_is_human,m.confirmed_automated_input,
       m.is_command,m.is_interrupted,m.is_system_reminder,
       m.parent_uuid,m.agent_id,m.subtype,m.has_conflict,
       m.host,m.source_platform,m.surface,m.source,m.kind,
       u.input_tokens,u.output_tokens,u.cache_read_tokens,u.cache_creation_tokens,
       u.cache_creation_5m,u.cache_creation_1h,u.service_tier,u.uuid IS NOT NULL AS usage_observed
FROM v_record_metadata m
LEFT JOIN usage u ON u.uuid=m.uuid
WHERE "#,
    // The metadata row carries both the record's and its session's columns.
    work_record_sql!(m, m),
    ";"
);

pub const USAGE_RECORDS: &str = concat!(
    "CREATE VIEW v_usage_records AS
WITH ",
    whitespace_sql!(),
    "
SELECT r.*, ",
    response_keyed_sql!(),
    " AS response_keyed
FROM v_records r CROSS JOIN whitespace WHERE type='assistant' AND usage_observed=1;"
);

pub const RESPONSE_USAGE: &str = concat!(
    "CREATE VIEW v_response_usage AS
SELECT t.*, 1 AS response_rank FROM v_usage_records t
WHERE ",
    response_selected_sql!(),
    ";"
);

pub const SESSION_EVENTS: &str = include_str!("../views/session_events.sql");

/// Every installed view, by name, in creation order: each after every view it
/// reads. Removal runs in the reverse order.
pub const ALL: [(&str, &str); 6] = [
    ("v_record_metadata", RECORD_METADATA),
    ("v_human_inputs", HUMAN_INPUTS),
    ("v_records", RECORDS),
    ("v_usage_records", USAGE_RECORDS),
    ("v_response_usage", RESPONSE_USAGE),
    ("v_session_events", SESSION_EVENTS),
];

#[cfg(test)]
mod tests {
    #[test]
    fn the_work_record_rule_reads_the_aliases_it_is_given() {
        // Every existing caller keeps the default r/s text byte for byte.
        assert_eq!(
            work_record_sql!(),
            "r.is_meta=0 AND s.kind='user' AND (r.model IS NULL OR r.model<>'<synthetic>')"
        );
        assert_eq!(
            work_record_sql!(m, m),
            "m.is_meta=0 AND m.kind='user' AND (m.model IS NULL OR m.model<>'<synthetic>')"
        );
    }
}
