CREATE VIEW v_response_usage AS
SELECT * FROM (
    SELECT *, row_number() OVER (
        PARTITION BY host,
          CASE WHEN response_keyed THEN 1 ELSE 0 END,
          CASE WHEN response_keyed THEN api_message_id ELSE uuid END,
          CASE WHEN response_keyed THEN request_id ELSE '' END
        ORDER BY ts_ms DESC, uuid DESC
    ) AS response_rank FROM v_usage_records
) WHERE response_rank=1;
