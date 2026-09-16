CREATE VIEW v_usage_records AS
SELECT * FROM v_records WHERE type='assistant' AND usage_observed=1;
