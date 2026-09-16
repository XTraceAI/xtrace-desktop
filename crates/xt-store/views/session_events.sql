CREATE VIEW v_session_events AS
SELECT * FROM v_records WHERE ts_ms IS NOT NULL;
