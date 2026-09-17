CREATE VIEW v_response_usage AS
SELECT t.*, 1 AS response_rank FROM v_usage_records t
WHERE coalesce(t.response_keyed,0)=0 OR NOT EXISTS (
    -- The caller can restrict candidate timestamps through records_ts, while
    -- successors are checked across all history through records_response.
    SELECT 1 FROM v_usage_records n
    WHERE n.host=t.host AND n.api_message_id=t.api_message_id AND n.request_id=t.request_id
      AND ((n.ts_ms IS NOT NULL AND t.ts_ms IS NULL)
        OR n.ts_ms>t.ts_ms
        OR (n.ts_ms IS t.ts_ms AND n.uuid>t.uuid))
);
