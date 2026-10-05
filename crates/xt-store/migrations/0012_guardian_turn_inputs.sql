-- A structural fact that one saved Codex user input is the input another
-- thread's turn dispatched to a Guardian reviewer, not one a person typed: the
-- reviewer's own turn was started by an exact turn of its parent thread, and
-- the input is the single user input of that reviewer turn. A caller that has
-- checked the saved rollout and its logs supplies these identities; the Store
-- accepts them as structure and does not claim to have verified those sources.
-- This row keeps only identities. No prompt, log line, body or digest of any of
-- them is stored.
--
-- This is a historical correction path for inputs the index already holds, not
-- automatic proof acquisition. It is deliberately a separate table: the parent
-- link is a turn, not a tool call, so it never enters the tool-call fields of
-- `confirmed_automated_inputs`, whose rows this migration leaves untouched.
--
-- One row confirms one existing input record in its owning Codex session, and
-- one Guardian turn confirms at most one input. Every identity has the closed
-- canonical shape its producer writes: lowercase hyphenated UUIDs, and the
-- owning session is `codex-` followed by its native identity. The parent
-- thread and its turn need not be indexed, so they are not foreign keys, but
-- they can never be the reviewer's own. Rows are immutable. The raw human
-- classification on `records` is left exactly as ingestion derived it; the
-- shared record projection reads either confirmation table as a durable
-- override of that one input's human eligibility.
CREATE TABLE guardian_turn_inputs (
    record_uuid TEXT PRIMARY KEY NOT NULL CHECK(record_uuid GLOB
        '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
    session_id TEXT NOT NULL CHECK(session_id = 'codex-' || native_session_id),
    native_session_id TEXT NOT NULL CHECK(native_session_id GLOB
        '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
    turn_id TEXT NOT NULL CHECK(turn_id GLOB
        '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
    parent_native_session_id TEXT NOT NULL CHECK(parent_native_session_id GLOB
        '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'
        AND parent_native_session_id <> native_session_id),
    parent_turn_id TEXT NOT NULL CHECK(parent_turn_id GLOB
        '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'
        AND parent_turn_id <> turn_id),
    evidence_kind TEXT NOT NULL CHECK(evidence_kind IN ('guardian_turn_dispatch')),
    matcher_version INTEGER NOT NULL CHECK(
        typeof(matcher_version) = 'integer' AND matcher_version > 0
    ),
    confirmed_at INTEGER NOT NULL CHECK(typeof(confirmed_at) = 'integer'),
    FOREIGN KEY(session_id) REFERENCES sessions(session_id),
    FOREIGN KEY(record_uuid, session_id) REFERENCES records(uuid, session_id),
    UNIQUE(native_session_id, turn_id)
);
CREATE TRIGGER guardian_turn_inputs_immutable_update
BEFORE UPDATE ON guardian_turn_inputs
BEGIN SELECT RAISE(ABORT, 'guardian turn input confirmation is immutable'); END;
CREATE TRIGGER guardian_turn_inputs_immutable_delete
BEFORE DELETE ON guardian_turn_inputs
BEGIN SELECT RAISE(ABORT, 'guardian turn input confirmation is immutable'); END;
