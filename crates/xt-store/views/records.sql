CREATE VIEW v_records AS
SELECT r.uuid,r.session_id,r.type,r.ts_ms,r.api_message_id,r.request_id,
       r.is_sidechain,r.role,r.model,r.is_tool_result_carrier,r.text_len,
       r.tool_use_count,r.is_human,r.is_command,r.is_interrupted,r.is_system_reminder,
       r.parent_uuid,r.agent_id,r.subtype,r.has_conflict,
       s.host,s.source_platform,s.surface,s.source,s.kind,
       u.input_tokens,u.output_tokens,u.cache_read_tokens,u.cache_creation_tokens,
       u.cache_creation_5m,u.cache_creation_1h,u.service_tier
FROM records r JOIN sessions s ON s.session_id=r.session_id
LEFT JOIN usage u ON u.uuid=r.uuid
WHERE r.is_meta=0 AND s.kind='user' AND (r.model IS NULL OR r.model<>'<synthetic>');
