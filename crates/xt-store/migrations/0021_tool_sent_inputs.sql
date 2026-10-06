-- A source fact that one saved Codex or Cursor user input was written by the
-- tool, not typed by a person. The pinned reader recognises each kind from a
-- structural marker in the native source, never from text, and passes a
-- metadata-only claim beside the canonical record it converted from that same
-- native row (`--automated-input-evidence`, contract `xtrace.automated_input`
-- version 1):
--
--   codex_subagent_notification  every content kind of the Codex user item is
--                                `multi_agent.subagent_notification`;
--   codex_turn_aborted           every content kind is `generic.turn_aborted`;
--   codex_apps_open_page         every content kind is
--                                `additional_content.codex_apps_open_page`;
--   cursor_conversation_summary  the Cursor message's own
--                                `providerOptions.cursor.isSummary` is true;
--   cursor_import_banner         the `[Imported from Cursor ...]` record the
--                                reader itself writes first in every session.
--
-- This row keeps only the record's identity and the kind. No prompt, summary,
-- notification, page, path or digest of any of them is stored.
--
-- Like `task_notification_inputs`, and unlike `injected_context_inputs`, a
-- proof may name a record the index already held: the claim is made from the
-- very native row whose record carries the UUID, and it binds only to a
-- stored, unconflicted, human-classified user input with that UUID in the
-- claimed session of the kind's own host. Codex and Cursor sessions keep no
-- resume checkpoint: every scan reads every session through the pinned reader
-- again, so the first scan after this migration corrects the inputs indexed
-- before it without any reset here.
--
-- One row names one input record in its owning session. Rows are immutable.
-- The raw human classification on `records` is left exactly as ingestion
-- derived it; the shared record projection reads this table, like the
-- confirmation, injected-context and task-notification tables, as a durable
-- override of that one input's human eligibility.
CREATE TABLE tool_sent_inputs (
    record_uuid TEXT PRIMARY KEY NOT NULL CHECK(
        length(trim(record_uuid)) > 0 AND length(record_uuid) <= 256
    ),
    session_id TEXT NOT NULL,
    evidence_kind TEXT NOT NULL CHECK(evidence_kind IN (
        'codex_subagent_notification',
        'codex_turn_aborted',
        'codex_apps_open_page',
        'cursor_conversation_summary',
        'cursor_import_banner'
    )),
    rule_version INTEGER NOT NULL CHECK(typeof(rule_version) = 'integer' AND rule_version = 1),
    FOREIGN KEY(record_uuid, session_id) REFERENCES records(uuid, session_id)
);
CREATE TRIGGER tool_sent_inputs_immutable_update
BEFORE UPDATE ON tool_sent_inputs
BEGIN SELECT RAISE(ABORT, 'tool-sent input proof is immutable'); END;
CREATE TRIGGER tool_sent_inputs_immutable_delete
BEFORE DELETE ON tool_sent_inputs
BEGIN SELECT RAISE(ABORT, 'tool-sent input proof is immutable'); END;
