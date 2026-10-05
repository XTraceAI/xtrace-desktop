CREATE VIEW v_records AS
SELECT r.uuid,r.session_id,r.type,r.ts,r.ts_ms,r.api_message_id,r.request_id,
       r.is_sidechain,r.role,r.model,r.is_tool_result_carrier,r.text_len,
       r.tool_use_count,
       h.human_is_eligible,h.human_text_len,h.human_excluded,
       -- Effective M-02 eligibility: a structurally confirmed automated input,
       -- from either confirmation table, a source-proven injected context
       -- input or a Claude Code task notification, is not a human message.
       -- raw_is_human keeps ingestion's classification.
       CASE WHEN a.record_uuid IS NULL AND g.record_uuid IS NULL AND i.record_uuid IS NULL
                AND n.record_uuid IS NULL
           THEN r.is_human ELSE 0 END AS is_human,
       r.is_human AS raw_is_human,
       (a.record_uuid IS NOT NULL OR g.record_uuid IS NOT NULL OR i.record_uuid IS NOT NULL
           OR n.record_uuid IS NOT NULL) AS confirmed_automated_input,
       r.is_command,r.is_interrupted,r.is_system_reminder,
       r.parent_uuid,r.agent_id,r.subtype,r.has_conflict,
       s.host,s.source_platform,s.surface,s.source,s.kind,
       u.input_tokens,u.output_tokens,u.cache_read_tokens,u.cache_creation_tokens,
       u.cache_creation_5m,u.cache_creation_1h,u.service_tier,u.uuid IS NOT NULL AS usage_observed
FROM records r JOIN sessions s ON s.session_id=r.session_id
JOIN v_human_inputs h ON h.uuid=r.uuid
LEFT JOIN usage u ON u.uuid=r.uuid
LEFT JOIN confirmed_automated_inputs a
    ON a.record_uuid=r.uuid AND a.session_id=r.session_id AND r.is_human=1
LEFT JOIN guardian_turn_inputs g
    ON g.record_uuid=r.uuid AND g.session_id=r.session_id AND r.is_human=1
LEFT JOIN injected_context_inputs i
    ON i.record_uuid=r.uuid AND i.session_id=r.session_id AND r.is_human=1
LEFT JOIN task_notification_inputs n
    ON n.record_uuid=r.uuid AND n.session_id=r.session_id AND r.is_human=1
WHERE r.is_meta=0 AND s.kind='user' AND (r.model IS NULL OR r.model<>'<synthetic>');
