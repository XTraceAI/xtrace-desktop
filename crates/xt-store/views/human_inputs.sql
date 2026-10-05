CREATE VIEW v_human_inputs AS
SELECT r.uuid,
 CASE WHEN a.record_uuid IS NOT NULL OR g.record_uuid IS NOT NULL OR i.record_uuid IS NOT NULL
           OR n.record_uuid IS NOT NULL
           OR (coalesce(r.role='user',0) AND (c.child_session_id IS NOT NULL OR o.session_id IS NOT NULL))
           OR (h.record_uuid IS NOT NULL AND h.retained_length IS NULL)
      THEN 0 ELSE r.is_human END AS human_is_eligible,
 CASE WHEN h.record_uuid IS NOT NULL THEN h.retained_length ELSE r.text_len END AS human_text_len,
 (a.record_uuid IS NOT NULL OR g.record_uuid IS NOT NULL OR i.record_uuid IS NOT NULL
    OR n.record_uuid IS NOT NULL
    OR (coalesce(r.role='user',0) AND (c.child_session_id IS NOT NULL OR o.session_id IS NOT NULL))
    OR (h.record_uuid IS NOT NULL AND h.retained_length IS NULL)) AS human_excluded
FROM records r JOIN sessions s ON s.session_id=r.session_id
LEFT JOIN confirmed_automated_inputs a ON a.record_uuid=r.uuid AND a.session_id=r.session_id
LEFT JOIN guardian_turn_inputs g ON g.record_uuid=r.uuid AND g.session_id=r.session_id
LEFT JOIN injected_context_inputs i ON i.record_uuid=r.uuid AND i.session_id=r.session_id
LEFT JOIN task_notification_inputs n ON n.record_uuid=r.uuid AND n.session_id=r.session_id
LEFT JOIN session_creation_relations c ON c.child_session_id=r.session_id AND c.child_host=s.host
    AND c.child_native_session_id=s.native_session_id AND c.state='accepted'
LEFT JOIN human_session_origins o ON o.session_id=r.session_id AND o.host=s.host
    AND o.native_session_id=s.native_session_id AND o.conflicted=0
LEFT JOIN human_input_adjustments h ON h.record_uuid=r.uuid AND h.session_id=r.session_id
    AND h.original_ts=r.ts AND h.original_length=r.text_len AND h.conflicted=0 AND r.has_conflict=0;
