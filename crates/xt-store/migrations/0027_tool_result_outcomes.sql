-- What each tool result said about how its call ended, kept as metadata only,
-- so the failed-call and declined-call rules (M-21) work whatever the content
-- retention setting says. Both facts are read at write time from the record's
-- own blocks, before retention decides about them (src/tool_result.rs):
--
-- * `tool_uses.call_id`: the host's own identifier of the call (a `tool_use`
--   block's `id`), the only link a tool result names. `call_id_conflict` is
--   set when two observations of the same call stated different identifiers;
--   such a call links to nothing.
-- * `tool_results`: one row per `tool_result` block, holding source facts
--   only, never a decision: which call it answers, its `is_error` flag (1 or
--   0; NULL when the block has none) and `flag_stated` (whether the block has
--   an `is_error` field at all, so a flag that is neither true nor false is
--   NULL with `flag_stated` 1), and `wording`, which of a few fixed host
--   messages the result's text starts with: `declined` (Claude Code's "The
--   user doesn't want to proceed with this tool use"), `interrupted`
--   ("[Request interrupted by user for tool use]"), `permission_denied`
--   ("Permission for this action was denied", or "Permission to use …" with
--   "has been denied") or `other`. No result text, no
--   argument and no output is kept. How the call ended is decided from these
--   facts by the reader, in Rust, so a corrected rule needs no rewrite here.
--
-- `fact_version` names the write-time derivation of these facts. A later
-- observation with a higher version replaces the row; one with the same
-- version that states other facts marks it `conflicted`, which readers treat
-- as unknown; a lower version changes nothing. Nothing is backfilled: records
-- indexed before this migration gain a row only when their source is read
-- again.
ALTER TABLE tool_uses ADD COLUMN call_id TEXT CHECK(
    call_id IS NULL OR (typeof(call_id) = 'text' AND length(trim(call_id)) > 0
        AND length(call_id) <= 256)
);
ALTER TABLE tool_uses ADD COLUMN call_id_conflict INTEGER NOT NULL DEFAULT 0 CHECK(
    call_id_conflict IN (0, 1)
);
CREATE INDEX tool_uses_call_id ON tool_uses(session_id, call_id) WHERE call_id IS NOT NULL;
CREATE TABLE tool_results (
    record_uuid TEXT NOT NULL CHECK(length(trim(record_uuid)) > 0),
    session_id TEXT NOT NULL,
    block_index INTEGER NOT NULL CHECK(typeof(block_index) = 'integer' AND block_index >= 0),
    call_id TEXT CHECK(
        call_id IS NULL OR (typeof(call_id) = 'text' AND length(trim(call_id)) > 0
            AND length(call_id) <= 256)
    ),
    is_error INTEGER CHECK(is_error IS NULL OR is_error IN (0, 1)),
    flag_stated INTEGER NOT NULL CHECK(flag_stated IN (0, 1)),
    wording TEXT NOT NULL CHECK(
        wording IN ('declined', 'interrupted', 'permission_denied', 'other')
    ),
    fact_version INTEGER NOT NULL CHECK(typeof(fact_version) = 'integer' AND fact_version >= 1),
    conflicted INTEGER NOT NULL DEFAULT 0 CHECK(conflicted IN (0, 1)),
    CHECK(flag_stated = 1 OR is_error IS NULL),
    PRIMARY KEY(record_uuid, block_index),
    FOREIGN KEY(record_uuid, session_id) REFERENCES records(uuid, session_id)
);
CREATE INDEX tool_results_session ON tool_results(session_id);
