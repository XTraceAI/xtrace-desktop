CREATE VIEW v_response_usage AS
WITH RECURSIVE later(before_uuid,after_uuid) AS (
    SELECT before_uuid,after_uuid FROM native_response_order
    UNION
    SELECT l.before_uuid,e.after_uuid FROM later l JOIN native_response_order e ON e.before_uuid=l.after_uuid
), timed AS (
    SELECT *, dense_rank() OVER (
        PARTITION BY host,
          CASE WHEN response_keyed THEN 1 ELSE 0 END,
          CASE WHEN response_keyed THEN api_message_id ELSE uuid END,
          CASE WHEN response_keyed THEN request_id ELSE '' END
        ORDER BY ts_ms DESC
    ) AS timestamp_rank FROM v_usage_records
), latest AS (
    SELECT t.* FROM timed t WHERE timestamp_rank=1 AND NOT EXISTS (
        SELECT 1 FROM later l JOIN timed n ON n.uuid=l.after_uuid
        WHERE l.before_uuid=t.uuid AND t.response_keyed AND n.response_keyed
          AND n.host=t.host AND n.api_message_id=t.api_message_id AND n.request_id=t.request_id
          AND n.timestamp_rank=1 AND n.ts_ms IS t.ts_ms
    )
)
SELECT * FROM (
    SELECT *, row_number() OVER (
        PARTITION BY host,
          CASE WHEN response_keyed THEN 1 ELSE 0 END,
          CASE WHEN response_keyed THEN api_message_id ELSE uuid END,
          CASE WHEN response_keyed THEN request_id ELSE '' END
        ORDER BY uuid DESC
    ) AS response_rank FROM latest
) WHERE response_rank=1;
