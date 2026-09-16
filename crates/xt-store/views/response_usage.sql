CREATE VIEW v_response_usage AS
SELECT * FROM (
    SELECT *, row_number() OVER (
        PARTITION BY host,
          CASE WHEN host='claude' AND length(trim(api_message_id))>0 AND length(trim(request_id))>0 THEN 1 ELSE 0 END,
          CASE WHEN host='claude' AND length(trim(api_message_id))>0 AND length(trim(request_id))>0 THEN api_message_id ELSE uuid END,
          CASE WHEN host='claude' AND length(trim(api_message_id))>0 AND length(trim(request_id))>0 THEN request_id ELSE '' END
        ORDER BY ts_ms DESC,uuid DESC
    ) AS response_rank FROM v_usage_records
) WHERE response_rank=1;
