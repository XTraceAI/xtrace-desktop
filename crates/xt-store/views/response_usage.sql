CREATE VIEW v_response_usage AS
SELECT t.*, 1 AS response_rank FROM v_usage_records t
WHERE coalesce(t.response_keyed,0)=0 OR NOT EXISTS (
    -- The caller can restrict candidate timestamps through records_ts, while
    -- successors are checked across all history through records_response.
    SELECT 1 FROM v_usage_records n
    WHERE n.host=t.host AND n.api_message_id=t.api_message_id AND n.request_id=t.request_id
      AND (xt_timestamp_cmp(n.ts,t.ts)>0
        OR (xt_timestamp_cmp(n.ts,t.ts)=0 AND n.uuid>t.uuid))
);
