-- A source fact that one saved Claude Code user input is a task notification:
-- the message Claude Code itself writes into the conversation when a
-- background agent, command or monitor it started finishes, not text a person
-- submitted. Claude Code marks the native line structurally
-- (`origin.kind = "task-notification"`); the native line's own marker is the
-- evidence, never its text. This row keeps only the record's identity. No
-- prompt, summary, result, path or digest of any of them is stored.
--
-- Unlike `injected_context_inputs`, a proof may name a record the index already
-- held: the marker is part of the very line whose UUID the record carries, and
-- the proof binds only to a stored, unconflicted, human-classified user input
-- with that UUID. That is how the inputs indexed before this migration are
-- corrected: the next native scan replays every Claude transcript once (the
-- checkpoint reset below, as migrations 5 to 7 did), and each task
-- notification it reads again gets its row.
--
-- One row names one input record in its owning session. Rows are immutable.
-- The raw human classification on `records` is left exactly as ingestion
-- derived it (replay only fills unknown facts, so it could not be rewritten
-- anyway); the shared record projection reads this table, like the
-- confirmation and injected-context tables, as a durable override of that
-- one input's human eligibility.
CREATE TABLE task_notification_inputs (
    record_uuid TEXT PRIMARY KEY NOT NULL CHECK(
        length(trim(record_uuid)) > 0 AND length(record_uuid) <= 256
    ),
    session_id TEXT NOT NULL,
    evidence_kind TEXT NOT NULL CHECK(evidence_kind IN ('claude_task_notification')),
    rule_version INTEGER NOT NULL CHECK(typeof(rule_version) = 'integer' AND rule_version = 1),
    FOREIGN KEY(record_uuid, session_id) REFERENCES records(uuid, session_id)
);
CREATE TRIGGER task_notification_inputs_immutable_update
BEFORE UPDATE ON task_notification_inputs
BEGIN SELECT RAISE(ABORT, 'task notification input proof is immutable'); END;
CREATE TRIGGER task_notification_inputs_immutable_delete
BEFORE DELETE ON task_notification_inputs
BEGIN SELECT RAISE(ABORT, 'task notification input proof is immutable'); END;
-- Prove unchanged Claude files again once, so the inputs indexed before this
-- migration are recognised. Reader-host checkpoints, records, receipts and
-- coverage are untouched, and a checkpoint recreated after this upgrade is
-- never reset again.
DELETE FROM native_checkpoints WHERE source='transcript';
