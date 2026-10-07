CREATE VIEW v_human_inputs AS
SELECT uuid,human_is_eligible,human_text_len,human_excluded FROM v_record_metadata;
