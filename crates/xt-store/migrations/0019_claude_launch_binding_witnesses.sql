-- One-use Codex variable binding witnesses, in the existing launch rows.
-- The bound key, code, output and UUID recipe are deliberately not stored.
ALTER TABLE claude_launch_candidates
    ADD COLUMN binding_call_offset INTEGER CHECK(binding_call_offset IS NULL OR
        (typeof(binding_call_offset)='integer' AND binding_call_offset >= 0));
ALTER TABLE claude_launch_candidates
    ADD COLUMN binding_output_offset INTEGER CHECK(binding_output_offset IS NULL OR
        (typeof(binding_output_offset)='integer' AND binding_output_offset > binding_call_offset));
ALTER TABLE claude_launch_staged_candidates
    ADD COLUMN binding_call_offset INTEGER CHECK(binding_call_offset IS NULL OR
        (typeof(binding_call_offset)='integer' AND binding_call_offset >= 0));
ALTER TABLE claude_launch_staged_candidates
    ADD COLUMN binding_output_offset INTEGER CHECK(binding_output_offset IS NULL OR
        (typeof(binding_output_offset)='integer' AND binding_output_offset > binding_call_offset));
