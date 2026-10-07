CREATE VIEW v_record_metadata AS
-- Internal: every stored record with its owning session and its stored input
-- facts, joined once, and the decisions read from those facts, each made in
-- one place. Unfiltered: v_human_inputs exposes this whole domain and
-- v_records the user-session, non-meta, non-synthetic part of it.
SELECT d.*,
       CASE WHEN d.human_excluded THEN 0 ELSE d.raw_is_human END AS human_is_eligible,
       -- Effective M-02 eligibility: a structurally confirmed automated input,
       -- from either confirmation table, a source-proven injected context
       -- input, a Claude Code task notification or a Codex or Cursor input
       -- the tool itself wrote, is not a human message.
       -- raw_is_human keeps ingestion's classification.
       CASE WHEN d.raw_is_human=1 AND d.automated_input THEN 0 ELSE d.raw_is_human END AS is_human,
       (coalesce(d.raw_is_human=1,0) AND d.automated_input) AS confirmed_automated_input
FROM (
    SELECT f.*,
           (f.automated_input OR f.launched_input OR f.withheld_text) AS human_excluded
    FROM (
        SELECT r.uuid,r.session_id,r.type,r.ts,r.ts_ms,r.api_message_id,r.request_id,
               r.is_meta,r.is_sidechain,r.role,r.model,r.is_tool_result_carrier,r.text_len,
               r.tool_use_count,r.is_human AS raw_is_human,
               r.is_command,r.is_interrupted,r.is_system_reminder,
               r.parent_uuid,r.agent_id,r.subtype,r.has_conflict,
               s.host,s.source_platform,s.surface,s.source,s.kind,
               (a.record_uuid IS NOT NULL OR g.record_uuid IS NOT NULL OR i.record_uuid IS NOT NULL
                   OR n.record_uuid IS NOT NULL OR t.record_uuid IS NOT NULL) AS automated_input,
               (coalesce(r.role='user',0)
                   AND (c.child_session_id IS NOT NULL OR o.session_id IS NOT NULL)) AS launched_input,
               (h.record_uuid IS NOT NULL AND h.retained_length IS NULL) AS withheld_text,
               CASE WHEN h.record_uuid IS NOT NULL THEN h.retained_length ELSE r.text_len END
                   AS human_text_len
        FROM records r JOIN sessions s ON s.session_id=r.session_id
        LEFT JOIN confirmed_automated_inputs a ON a.record_uuid=r.uuid AND a.session_id=r.session_id
        LEFT JOIN guardian_turn_inputs g ON g.record_uuid=r.uuid AND g.session_id=r.session_id
        LEFT JOIN injected_context_inputs i ON i.record_uuid=r.uuid AND i.session_id=r.session_id
        LEFT JOIN task_notification_inputs n ON n.record_uuid=r.uuid AND n.session_id=r.session_id
        LEFT JOIN tool_sent_inputs t ON t.record_uuid=r.uuid AND t.session_id=r.session_id
        LEFT JOIN session_creation_relations c ON c.child_session_id=r.session_id AND c.child_host=s.host
            AND c.child_native_session_id=s.native_session_id AND c.state='accepted'
        LEFT JOIN human_session_origins o ON o.session_id=r.session_id AND o.host=s.host
            AND o.native_session_id=s.native_session_id AND o.conflicted=0
        LEFT JOIN human_input_adjustments h ON h.record_uuid=r.uuid AND h.session_id=r.session_id
            AND h.original_ts=r.ts AND h.original_length=r.text_len AND h.conflicted=0 AND r.has_conflict=0
    ) f
) d;
