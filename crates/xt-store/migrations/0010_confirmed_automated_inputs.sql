-- A structural fact that one saved user input was submitted by another agent,
-- not by a person: the parent agent's tool call launched or resumed the target
-- session, its paired successful result named that session, and the target's
-- saved input is exactly what the call submitted. The comparison happens in
-- memory elsewhere; this row keeps only the identities that proved it. No
-- prompt, command, output body or digest of any of them is stored.
--
-- One row confirms one existing input record in its owning session. The raw
-- human classification on `records` is left exactly as ingestion derived it,
-- so replay keeps its fill-only behaviour; the shared record projection reads
-- this row as a durable override of that input's human eligibility. The parent
-- session and its call need not be indexed, so they are not foreign keys.
--
-- Rows are immutable. A second, different proof for the same input, or one
-- proof naming two inputs, is refused rather than transferred. Receipts,
-- checkpoints, records and their UUIDs are untouched: no replay is requested.
CREATE TABLE confirmed_automated_inputs (
    record_uuid TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL,
    native_session_id TEXT NOT NULL CHECK(
        length(trim(native_session_id)) > 0 AND length(native_session_id) <= 256
    ),
    evidence_kind TEXT NOT NULL CHECK(evidence_kind IN ('agent_dispatch')),
    matcher_version INTEGER NOT NULL CHECK(
        typeof(matcher_version) = 'integer' AND matcher_version > 0
    ),
    parent_host TEXT NOT NULL CHECK(parent_host IN ('claude', 'codex', 'cursor', 'other')),
    parent_session_id TEXT NOT NULL CHECK(
        length(trim(parent_session_id)) > 0 AND length(parent_session_id) <= 256
    ),
    parent_tool_call_id TEXT NOT NULL CHECK(
        length(trim(parent_tool_call_id)) > 0 AND length(parent_tool_call_id) <= 256
    ),
    parent_operation_index INTEGER NOT NULL CHECK(
        typeof(parent_operation_index) = 'integer' AND parent_operation_index >= 0
    ),
    parent_result_id TEXT CHECK(
        parent_result_id IS NULL
        OR (length(trim(parent_result_id)) > 0 AND length(parent_result_id) <= 256)
    ),
    confirmed_at INTEGER NOT NULL CHECK(typeof(confirmed_at) = 'integer'),
    FOREIGN KEY(record_uuid, session_id) REFERENCES records(uuid, session_id),
    UNIQUE(parent_host, parent_session_id, parent_tool_call_id, parent_operation_index)
);
CREATE TRIGGER confirmed_automated_inputs_immutable_update
BEFORE UPDATE ON confirmed_automated_inputs
BEGIN SELECT RAISE(ABORT, 'automated input confirmation is immutable'); END;
CREATE TRIGGER confirmed_automated_inputs_immutable_delete
BEFORE DELETE ON confirmed_automated_inputs
BEGIN SELECT RAISE(ABORT, 'automated input confirmation is immutable'); END;
