-- A short preview of the words of two kinds of saved user input, kept so the
-- Dashboard's activity bubble can show them without reading the session's
-- original file on every hover. This is the one place the index keeps text
-- whatever the content retention setting says, and only this much of it:
--
-- * `person`: a message the stored M-02 classification called a person's
--   when it was indexed, and whose whole text is theirs (no human-input
--   adjustment says only part of it is). The text is the record's `text`
--   blocks joined exactly as the classifier joins them.
-- * `automatic`: a Claude Code task notification a stored source proof names
--   (`task_notification_inputs`), and only its own `<summary>`.
--
-- `text` is one line, whitespace runs collapsed, at most 280 characters;
-- `truncated` says whether more followed. Nothing else of the record is kept:
-- no tool result, tool input, title, path or digest.
--
-- Readers never trust a row on its own: they choose the record by the current
-- classification first and only then look up its preview, so a record that a
-- later proof makes ineligible is never shown as a person's message.
--
-- One row names one record in its owning session and is filled once; a
-- replay never rewrites it. "Delete stored content" clears `text` (the row
-- then counts as no preview), and a human-input adjustment that arrives later
-- removes a person preview it contradicts.
CREATE TABLE record_previews (
    record_uuid TEXT PRIMARY KEY NOT NULL CHECK(
        length(trim(record_uuid)) > 0 AND length(record_uuid) <= 256
    ),
    session_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('person','automatic')),
    text TEXT CHECK(text IS NULL OR (typeof(text) = 'text' AND length(text) <= 280)),
    truncated INTEGER NOT NULL CHECK(truncated IN (0,1)),
    rule_version INTEGER NOT NULL CHECK(typeof(rule_version) = 'integer' AND rule_version = 1),
    FOREIGN KEY(record_uuid, session_id) REFERENCES records(uuid, session_id)
);
-- Fill the previews of history indexed before this migration: the next native
-- scan replays every Claude transcript once (the checkpoint reset below, as
-- migration 16 did), and each eligible input it reads again gets its row.
-- Codex and Cursor are read whole by the pinned reader on every scan, so they
-- fill without a reset. Records, receipts and coverage are untouched, and a
-- checkpoint recreated after this upgrade is never reset again.
DELETE FROM native_checkpoints WHERE source='transcript';
