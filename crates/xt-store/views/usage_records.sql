CREATE VIEW v_usage_records AS
WITH whitespace(chars) AS (
    -- Unicode White_Space, matching Rust str::trim used for blank input.
    SELECT char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)
)
SELECT r.*, host='claude' AND trim(api_message_id,chars)<>''
    AND trim(request_id,chars)<>'' AS response_keyed
FROM v_records r CROSS JOIN whitespace WHERE type='assistant' AND usage_observed=1;
