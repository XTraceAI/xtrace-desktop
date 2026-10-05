-- A source fact that one saved Codex user input is context Codex itself
-- injected, not text a person submitted: the body Codex adds when a skill is
-- selected. The native reader declares it so from the row's own content kinds
-- and returns this row's identities beside the canonical record it converted
-- from that row. This row keeps only those identities. No prompt, body, path,
-- title or digest of any of them is stored.
--
-- A proof is accepted only together with its record, from the same exact read
-- of one session, in the ingest transaction that first stores that record. It
-- is never attached to a row the index already held: a later read of an
-- existing record is not evidence of how that earlier row was built. This is a
-- separate table beside `confirmed_automated_inputs` and
-- `guardian_turn_inputs`, whose rows this migration leaves untouched.
--
-- One row names one input record in its owning Codex session, and one native
-- item or native row names at most one input. Every identity has the closed
-- shape the reader writes: lowercase hyphenated UUIDs, `msg_` item IDs, and
-- the owning session is `codex-` followed by its native identity. A flat
-- rollout has no rollout ID; a paginated one always has one. The contract,
-- version and kind are closed. Rows are immutable. The raw human
-- classification on `records` is left exactly as ingestion derived it; the
-- shared record projection reads this table, like either confirmation table,
-- as a durable override of that one input's human eligibility. The trigger
-- below refuses a proof for an input either confirmation table already holds;
-- those tables have no matching trigger, so the reverse order is refused only
-- by the Store's Rust confirmation paths, not by SQL.
CREATE TABLE injected_context_inputs (
    record_uuid TEXT PRIMARY KEY NOT NULL CHECK(record_uuid GLOB
        '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
    session_id TEXT NOT NULL CHECK(session_id = 'codex-' || native_session_id),
    native_session_id TEXT NOT NULL CHECK(native_session_id GLOB
        '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
    evidence_contract TEXT NOT NULL CHECK(evidence_contract IN ('memhub.codex.origin_evidence')),
    evidence_version INTEGER NOT NULL CHECK(
        typeof(evidence_version) = 'integer' AND evidence_version IN (1)
    ),
    evidence_kind TEXT NOT NULL CHECK(evidence_kind IN ('codex_selected_skill_instructions')),
    segment_history TEXT NOT NULL CHECK(segment_history IN ('flat', 'paginated')),
    rollout_id TEXT CHECK(
        (segment_history = 'flat' AND rollout_id IS NULL)
        OR (segment_history = 'paginated' AND rollout_id IS NOT NULL AND rollout_id GLOB
        '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]')
    ),
    row_index INTEGER NOT NULL CHECK(typeof(row_index) = 'integer' AND row_index >= 0),
    row_ordinal INTEGER CHECK(
        row_ordinal IS NULL OR (typeof(row_ordinal) = 'integer' AND row_ordinal >= 0)
    ),
    -- `msg_` then a lowercase hyphenated UUID, or `msg_` then 50 lowercase hex.
    item_id TEXT NOT NULL CHECK(
        substr(item_id, 1, 4) = 'msg_' AND (
            (length(item_id) = 40 AND substr(item_id, 5) GLOB
            '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]')
            OR (length(item_id) = 54 AND substr(item_id, 5) NOT GLOB '*[^0-9a-f]*')
        )
    ),
    turn_id TEXT NOT NULL CHECK(turn_id GLOB
        '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
    -- The discovery instant of the read that inserted the record.
    observed_at INTEGER NOT NULL CHECK(typeof(observed_at) = 'integer'),
    FOREIGN KEY(session_id) REFERENCES sessions(session_id),
    FOREIGN KEY(record_uuid, session_id) REFERENCES records(uuid, session_id),
    UNIQUE(native_session_id, item_id)
);
-- One native row yields one record. A flat rollout has no rollout ID, which
-- a plain UNIQUE constraint would treat as distinct from every other.
CREATE UNIQUE INDEX injected_context_inputs_row
    ON injected_context_inputs(native_session_id, coalesce(rollout_id, ''), row_index);
CREATE TRIGGER injected_context_inputs_exclusive
BEFORE INSERT ON injected_context_inputs
WHEN EXISTS(SELECT 1 FROM confirmed_automated_inputs WHERE record_uuid=NEW.record_uuid)
    OR EXISTS(SELECT 1 FROM guardian_turn_inputs WHERE record_uuid=NEW.record_uuid)
BEGIN SELECT RAISE(ABORT, 'input is already confirmed'); END;
CREATE TRIGGER injected_context_inputs_immutable_update
BEFORE UPDATE ON injected_context_inputs
BEGIN SELECT RAISE(ABORT, 'injected context input proof is immutable'); END;
CREATE TRIGGER injected_context_inputs_immutable_delete
BEFORE DELETE ON injected_context_inputs
BEGIN SELECT RAISE(ABORT, 'injected context input proof is immutable'); END;
